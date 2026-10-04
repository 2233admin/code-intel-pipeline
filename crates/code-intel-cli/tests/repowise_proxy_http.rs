//! Public CLI/wire contracts from desk417 captures P1-P6 and R1-R2.
//! Expected bytes/digest come from the old CLI captures, not production helpers.
mod common;
#[path = "support/sha256.rs"]
mod sha256;

use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

static NEXT_ROOT: AtomicUsize = AtomicUsize::new(0);
const REMOTE_LINKS: &str = "/__code-intel/remote-links.json";
const ORIGIN: &str = "https://github.com/fixture-owner/fixture-repo.git";

struct TempRoot(PathBuf);
impl TempRoot {
    fn new() -> Self {
        loop {
            let path = std::env::temp_dir().join(format!(
                "code-intel-proxy-http-{}-{}",
                std::process::id(), NEXT_ROOT.fetch_add(1, Ordering::Relaxed)
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("create isolated root: {error}"),
            }
        }
    }
}
impl Drop for TempRoot {
    fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); }
}

struct ChildGuard(Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

struct Upstream {
    port: u16,
    stop: Arc<AtomicBool>,
    repos_calls: Arc<AtomicUsize>,
    thread: Option<JoinHandle<()>>,
}
impl Upstream {
    fn start(repos: Reply) -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        listener.set_nonblocking(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = Arc::clone(&stop);
        let repos_calls = Arc::new(AtomicUsize::new(0));
        let calls = Arc::clone(&repos_calls);
        let thread = thread::spawn(move || {
            while !stopped.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                        stream.set_write_timeout(Some(Duration::from_secs(5))).unwrap();
                        if let Ok(request) = read_request(&mut stream) {
                            if request.path == "/api/repos" {
                                repos.send(&mut stream);
                                calls.fetch_add(1, Ordering::Relaxed);
                            } else if let Some(reply) = fixture_reply(request) {
                                reply.send(&mut stream);
                            }
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(error) => panic!("upstream accept: {error}"),
                }
            }
        });
        Self { port, stop, repos_calls, thread: Some(thread) }
    }
}
impl Drop for Upstream {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let result = thread.join();
            if !thread::panicking() { result.unwrap(); }
        }
    }
}

struct Proxy {
    child: ChildGuard,
    _upstream: Upstream,
    port: u16,
}
impl Proxy {
    fn start(root: &TempRoot, repos: Reply) -> Self {
        let upstream = Upstream::start(repos);
        let reservation = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = reservation.local_addr().unwrap().port();
        let mut command = common::cli_in(&root.0);
        command.args(["repowise-proxy", &upstream.port.to_string(), &port.to_string()])
            .env("CODE_INTEL_DATA_ROOT", root.0.join("data"))
            .env("CODE_INTEL_LANG", "en")
            .env("HOME", &root.0).env("USERPROFILE", &root.0)
            .stdout(Stdio::null())
            .stderr(fs::File::create(root.0.join("proxy.stderr")).unwrap());
        for key in ["HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY", "NO_PROXY",
                    "http_proxy", "https_proxy", "all_proxy", "no_proxy"] {
            command.env_remove(key);
        }
        isolate_git(&mut command, root);
        drop(reservation);
        let mut proxy = Self { child: ChildGuard(command.spawn().unwrap()), _upstream: upstream, port };
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            assert!(proxy.child.0.try_wait().unwrap().is_none(), "proxy exited: {}",
                fs::read_to_string(root.0.join("proxy.stderr")).unwrap());
            if TcpStream::connect(("127.0.0.1", port)).is_ok() { break; }
            assert!(Instant::now() < deadline, "proxy did not listen");
            thread::sleep(Duration::from_millis(20));
        }
        proxy
    }

    fn request(&self, method: &str, path: &str, body: &[u8]) -> Reply {
        let mut stream = TcpStream::connect(("127.0.0.1", self.port)).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(15))).unwrap();
        stream.set_write_timeout(Some(Duration::from_secs(5))).unwrap();
        write!(stream, "{method} {path} HTTP/1.0\r\nHost: localhost\r\nConnection: close\r\nAuthorization: Bearer synthetic-private-marker\r\nCookie: private=synthetic\r\nContent-Length: {}\r\n\r\n", body.len()).unwrap();
        stream.write_all(body).unwrap();
        let mut reader = BufReader::new(stream);
        let mut first = String::new();
        reader.read_line(&mut first).unwrap();
        let status = first.split_whitespace().nth(1).unwrap().parse().unwrap();
        let headers = read_headers(&mut reader).unwrap();
        let mut body = Vec::new();
        reader.read_to_end(&mut body).unwrap();
        // HTTP/1.0 requests keep large bodies unchunked, independent of client libraries.
        assert!(!headers.iter().any(|(name, _)| name == "transfer-encoding"));
        if let Some((_, length)) = headers.iter().find(|(name, _)| name == "content-length") {
            assert_eq!(body.len(), length.parse::<usize>().unwrap());
        }
        Reply { status, headers, body }
    }
}

fn isolate_git(command: &mut Command, root: &TempRoot) {
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().to_ascii_uppercase().starts_with("GIT_") {
            command.env_remove(key);
        }
    }
    command.env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", root.0.join("absent-git-config"))
        .env("GIT_TERMINAL_PROMPT", "0");
}

struct Request {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}
fn read_headers(reader: &mut impl BufRead) -> std::io::Result<Vec<(String, String)>> {
    let mut headers = Vec::new();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 || line == "\r\n" { return Ok(headers); }
        let (name, value) = line.trim_end().split_once(':').expect("HTTP header");
        headers.push((name.to_ascii_lowercase(), value.trim().to_string()));
    }
}
fn read_request(stream: &mut TcpStream) -> std::io::Result<Request> {
    let mut reader = BufReader::new(stream);
    let mut first = String::new();
    reader.read_line(&mut first)?;
    let mut words = first.split_whitespace();
    let method = words.next().unwrap_or_default().to_string();
    let path = words.next().unwrap_or_default().to_string();
    let headers = read_headers(&mut reader)?;
    let length = headers.iter().find(|(name, _)| name == "content-length")
        .map(|(_, value)| value.parse::<usize>().unwrap()).unwrap_or(0);
    let mut body = vec![0; length];
    reader.read_exact(&mut body)?;
    Ok(Request { method, path, headers, body })
}

struct Reply {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}
impl Reply {
    fn new(status: u16, mime: &str, body: impl Into<Vec<u8>>) -> Self {
        Self { status, headers: vec![("Content-Type".into(), mime.into())], body: body.into() }
    }
    fn send(&self, stream: &mut TcpStream) {
        let result = (|| -> std::io::Result<()> {
            write!(stream, "HTTP/1.1 {} Fixture\r\nConnection: close\r\nContent-Length: {}\r\n", self.status, self.body.len())?;
            for (name, value) in &self.headers { write!(stream, "{name}: {value}\r\n")?; }
            stream.write_all(b"\r\n")?;
            stream.write_all(&self.body)
        })();
        // Redirect/body rejection can make the proxy close before consuming a response.
        if let Err(error) = result {
            assert!(matches!(error.kind(), std::io::ErrorKind::BrokenPipe | std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::ConnectionAborted), "upstream send: {error}");
        }
    }
}

fn fixture_reply(request: Request) -> Option<Reply> {
    let reject = |status| Some(Reply::new(status, "text/plain", b"fixture rejected request"));
    // This upstream denies leaked client credentials, rather than echoing headers.
    if request.headers.iter().any(|(name, _)| name == "authorization" || name == "cookie") {
        return reject(401);
    }
    let reply = match request.path.as_str() {
        "/fixture?q=1" if request.method == "POST" && request.body == b"\0\xffbinary\0body" =>
            Reply::new(200, "application/json", b"{\"channels\":[]}"),
        "/fixture?branch=body-get" if request.method == "GET" && request.body == b"nonempty-get-body" =>
            Reply::new(200, "application/json", b"{\"channels\":[]}"),
        "/fixture?branch=empty-post" | "/fixture?branch=empty-get" => {
            let method = if request.path.ends_with("empty-post") { "POST" } else { "GET" };
            // The fixture endpoint rejects ambiguous/changed empty-request framing.
            if request.method != method || !request.body.is_empty() || request.headers.iter()
                .any(|(name, _)| name == "content-length" || name == "transfer-encoding") {
                return reject(409);
            }
            Reply::new(200, "application/json", b"{\"channels\":[]}")
        }
        "/error/404" | "/error/500" => {
            let status = if request.path.ends_with("404") { 404 } else { 500 };
            let mut reply = Reply::new(status, "application/json; charset=utf-8",
                format!("{{\"error\": \"Dashboard\", \"status\": {status}}}"));
            reply.headers.extend([
                ("Set-Cookie".into(), "first=1; Path=/".into()),
                ("Set-Cookie".into(), "second=2; Path=/".into()),
                ("X-Upstream".into(), "retained".into()),
            ]);
            reply
        }
        "/binary-large" => Reply::new(200, "application/octet-stream",
            (0u8..=255).cycle().take(11_534_336).collect::<Vec<_>>()),
        "/invalid-json" => Reply::new(200, "application/json", b"{\"title\":\"valid\"}\xff"),
        "/invalid-html" => Reply::new(200, "text/html", b"<body>Dashboard\xff</body>"),
        "/transport" => return None,
        path if path.starts_with("/redirect/") => {
            let hops: u16 = path.rsplit('/').next().unwrap().parse().unwrap();
            if hops == 0 { Reply::new(200, "text/plain", b"redirect-complete") }
            else {
                let mut reply = Reply::new(302, "text/plain", b"redirect");
                reply.headers.push(("Location".into(), format!("/redirect/{}", hops - 1)));
                reply
            }
        }
        _ => return reject(400),
    };
    Some(reply)
}

#[test]
fn binary_post_and_get_bodies_reach_the_endpoint_without_client_credentials() {
    let root = TempRoot::new();
    let proxy = Proxy::start(&root, Reply::new(200, "application/json", b"[]"));
    for (method, path, body) in [
        ("POST", "/fixture?q=1", b"\0\xffbinary\0body".as_slice()),
        ("GET", "/fixture?branch=body-get", b"nonempty-get-body".as_slice()),
        ("POST", "/fixture?branch=empty-post", b"".as_slice()),
        ("GET", "/fixture?branch=empty-get", b"".as_slice()),
    ] {
        let reply = proxy.request(method, path, body);
        // P1/P2：端点只接受原始请求体、无凭据和旧版空请求 framing。
        assert_eq!(reply.status, 200, "{method} {path}");
        assert_eq!(reply.body, b"{\"channels\":[]}");
    }
}

#[test]
fn real_error_status_body_and_both_cookies_survive_the_proxy() {
    let root = TempRoot::new();
    let proxy = Proxy::start(&root, Reply::new(200, "application/json", b"[]"));
    for (path, status, expected) in [
        ("/error/404", 404, b"{\"error\":\"Dashboard\",\"status\":404}".as_slice()),
        ("/error/500", 500, b"{\"error\":\"Dashboard\",\"status\":500}".as_slice()),
    ] {
        let reply = proxy.request("GET", path, b"");
        // P3：真实 404/500 不是合成 502；两个独立 cookie 都保留。
        assert_eq!(reply.status, status);
        assert_eq!(reply.body, expected);
        let cookies: Vec<_> = reply.headers.iter().filter(|(name, _)| name == "set-cookie")
            .map(|(_, value)| value.as_str()).collect();
        assert_eq!(cookies, ["first=1; Path=/", "second=2; Path=/"]);
    }
}

#[test]
fn binary_over_ten_mebibytes_preserves_the_recorded_digest() {
    let root = TempRoot::new();
    let proxy = Proxy::start(&root, Reply::new(200, "application/json", b"[]"));
    let reply = proxy.request("GET", "/binary-large", b"");
    // P4：旧捕获的 11 MiB 字节序列及独立 SHA-256 字面值。
    assert_eq!(reply.status, 200);
    assert_eq!(reply.body.len(), 11_534_336);
    assert_eq!(sha256::sha256_hex(&reply.body),
        "50a42d7ae7229b8b04189593c38b80dfca5562706f2f3c6a6075077de33b29f6");
}

#[test]
fn invalid_utf8_is_not_lossily_forwarded_as_json_or_html() {
    let root = TempRoot::new();
    let proxy = Proxy::start(&root, Reply::new(200, "application/json", b"[]"));
    let json = proxy.request("GET", "/invalid-json", b"");
    // P4：整段 JSON 的严格 UTF-8 读取失败，旧版输出为空。
    assert_eq!(json.status, 200);
    assert_eq!(json.body, b"");
    let html = proxy.request("GET", "/invalid-html", b"");
    assert_eq!(html.status, 200);
    let text = String::from_utf8(html.body).unwrap();
    // P4：拒绝损坏 HTML 内容，仍保留公开 remote-links 注入功能。
    assert!(!text.contains("Dashboard") && !text.contains('\u{fffd}'));
    assert!(text.contains(REMOTE_LINKS));
}

#[test]
fn transport_failure_and_five_or_six_redirects_are_502_but_four_succeed() {
    let root = TempRoot::new();
    let proxy = Proxy::start(&root, Reply::new(200, "application/json", b"[]"));
    for (path, status, body) in [
        ("/transport", 502, b"Proxy error".as_slice()),
        ("/redirect/4", 200, b"redirect-complete".as_slice()),
        ("/redirect/5", 502, b"Proxy error".as_slice()),
        ("/redirect/6", 502, b"Proxy error".as_slice()),
    ] {
        let reply = proxy.request("GET", path, b"");
        // P5/P6：旧版最多成功跟随四跳；五跳、六跳及传输失败返回 502。
        assert_eq!(reply.status, status, "{path}");
        assert_eq!(reply.body, body, "{path}");
    }
}

#[test]
fn startup_warmup_resolves_a_real_git_origin_through_public_remote_links() {
    let root = TempRoot::new();
    let repo = root.0.join("git-fixture");
    fs::create_dir(&repo).unwrap();
    for args in [vec!["init", "--quiet"], vec!["remote", "add", "origin", ORIGIN]] {
        let mut command = Command::new("git");
        command.args(["-C", repo.to_str().unwrap()]).args(args);
        isolate_git(&mut command, &root);
        let output = command.output().unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    }
    let repos = serde_json::to_vec(&serde_json::json!([
        {"id": "fixture-repo", "local_path": repo}
    ])).unwrap();
    let proxy = Proxy::start(&root, Reply::new(200, "application/json", repos));
    let expected = serde_json::json!({"fixture-repo": {
        "host_type": "github", "web_base_url": "https://github.com/fixture-owner/fixture-repo"
    }});
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let reply = proxy.request("GET", REMOTE_LINKS, b"");
        assert_eq!(reply.status, 200);
        let actual: serde_json::Value = serde_json::from_slice(&reply.body).unwrap();
        // R1：仅通过公开 HTTP 路由观察真实 Git origin 与上游 repo ID 的关联。
        if actual == expected { break; }
        assert_eq!(actual, serde_json::json!({}));
        assert!(Instant::now() < deadline, "warmup never published fixture origin");
        thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn failed_malformed_or_empty_warmup_never_invents_remote_links() {
    for (status, body) in [(500, b"[]".as_slice()), (200, b"{".as_slice()), (200, b"[]".as_slice())] {
        let root = TempRoot::new();
        let proxy = Proxy::start(&root, Reply::new(status, "application/json", body));
        let deadline = Instant::now() + Duration::from_secs(10);
        // A second upstream attempt proves the first result was consumed and rejected.
        while proxy._upstream.repos_calls.load(Ordering::Relaxed) < 2 {
            assert!(Instant::now() < deadline, "warmup did not retry the unusable response");
            thread::sleep(Duration::from_millis(20));
        }
        let reply = proxy.request("GET", REMOTE_LINKS, b"");
        // R2：上游 500、损坏 JSON、空数组只能产生空链接映射。
        assert_eq!(reply.status, 200);
        assert_eq!(reply.body, b"{}");
    }
}
