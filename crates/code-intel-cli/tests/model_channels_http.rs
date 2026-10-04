//! Public CLI contracts recorded from the pre-migration M1–M6 HTTP captures.
//! Only application error categories are stable; HTTP-library diagnostics are not.

mod common;

use std::fs;
use std::io::ErrorKind;
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};
use tiny_http::{Header, Response, Server};

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let path = std::env::temp_dir().join(format!(
            "code-intel-model-http-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir(&path).unwrap();
        let fixture = Self(path);
        fs::write(
            fixture.0.join("request.json"),
            include_bytes!("fixtures/model-routing/ready-local.json"),
        )
        .unwrap();
        fixture
    }

    fn command(&self, endpoint: Option<&str>) -> Command {
        let mut command = common::cli_in(&self.0);
        command
            .env_clear()
            .env("HOME", &self.0)
            .env("USERPROFILE", &self.0)
            .env("APPDATA", &self.0)
            .env("LOCALAPPDATA", &self.0)
            .env("CODE_INTEL_DATA_ROOT", self.0.join("data"))
            .env("CODE_INTEL_LANG", "en")
            .args(["model", "route", "--request"])
            .arg(self.0.join("request.json"));
        if let Some(endpoint) = endpoint {
            command.env("CODE_INTEL_CC_SWITCH_ENDPOINT", endpoint);
        }
        command
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct Process(Option<Child>);

impl Process {
    fn finish(&mut self) -> Output {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if self.0.as_mut().unwrap().try_wait().unwrap().is_some() {
                return self.0.take().unwrap().wait_with_output().unwrap();
            }
            assert!(Instant::now() < deadline, "model route did not terminate");
            thread::sleep(Duration::from_millis(10));
        }
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        if let Some(child) = self.0.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn broker() -> (Server, String) {
    let server = Server::http("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}", server.server_addr());
    (server, endpoint)
}

fn run(mut command: Command, upstream: Option<(&Server, u16, Vec<u8>)>) -> Output {
    let mut process = Process(Some(
        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    ));
    if let Some((server, status, body)) = upstream {
        let request = server
            .recv_timeout(Duration::from_secs(10))
            .unwrap()
            .expect("model route must contact the controlled broker");
        let response = Response::from_data(body)
            .with_status_code(status)
            .with_header(Header::from_bytes("Content-Type", "application/json").unwrap());
        // Error-status consumers may close without reading the response body.
        let _ = request.respond(response);
    }
    process.finish()
}

fn assert_ready(output: &Output) {
    // M1、M2、M6：本地渠道成功时进程退出码为 0。
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    // 成功路由不输出应用错误。
    assert!(output.stderr.is_empty(), "{output:?}");
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    // 捕获结果使用公开的模型路由结果协议。
    assert_eq!(result["schema"], "code-intel-model-routing-result.v1");
    // HTTP 返回合法渠道列表时，已有本地渠道仍就绪。
    assert_eq!(result["status"], "ready");
    // 独立期望值来自旧 CLI 实际输出，而非转发参数。
    assert_eq!(result["selected"]["candidateId"], "ollama-local");
}

fn assert_failure(output: &Output, category: &str) {
    // M3、M4、M5：上游或凭证失败均以应用错误退出码 65 返回。
    assert_eq!(output.status.code(), Some(65), "{output:?}");
    // 失败不会发布伪造的就绪路由。
    assert!(output.stdout.is_empty(), "{output:?}");
    // 只锁定应用错误类别，不锁定第三方库的错误措辞。
    assert!(
        String::from_utf8_lossy(&output.stderr).starts_with(category),
        "{output:?}"
    );
}

#[test]
fn no_broker_keeps_the_existing_local_route() {
    let fixture = Fixture::new();
    assert_ready(&run(fixture.command(None), None));
}

#[test]
fn broker_200_with_valid_channels_keeps_the_existing_local_route() {
    let fixture = Fixture::new();
    let (server, endpoint) = broker();
    let output = run(
        fixture.command(Some(&endpoint)),
        Some((&server, 200, br#"{"channels":[]}"#.to_vec())),
    );
    assert_ready(&output);
}

#[test]
fn malformed_and_missing_channels_report_distinct_application_errors() {
    for (body, category) in [
        ("{", "CC Switch response parse failed:"),
        ("{}", "CC Switch response missing 'channels' array"),
    ] {
        let fixture = Fixture::new();
        let (server, endpoint) = broker();
        let output = run(
            fixture.command(Some(&endpoint)),
            Some((&server, 200, body.as_bytes().to_vec())),
        );
        assert_failure(&output, category);
    }
}

#[test]
fn non_200_success_and_http_errors_are_not_json_parse_failures() {
    for (status, category) in [
        (201, "CC Switch returned status 201"),
        (404, "CC Switch request failed:"),
        (500, "CC Switch request failed:"),
    ] {
        let fixture = Fixture::new();
        let (server, endpoint) = broker();
        let output = run(
            fixture.command(Some(&endpoint)),
            Some((&server, status, br#"{"channels":[]}"#.to_vec())),
        );
        assert_failure(&output, category);
    }
}

#[test]
fn a_key_over_http_is_refused_before_any_connection() {
    let fixture = Fixture::new();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let mut command = fixture.command(Some(&endpoint));
    command.env("CODE_INTEL_CC_SWITCH_API_KEY", "synthetic-not-a-secret");
    let output = run(command, None);
    assert_failure(&output, "CODE_INTEL_CC_SWITCH_API_KEY is set");
    // M5：HTTP 明文连接被拒绝，而非尝试请求后报告网络错误。
    assert!(String::from_utf8_lossy(&output.stderr)
        .contains("refusing to send the key over an unencrypted connection"));
    // 监听端口始终保留；连 TCP 连接也不能到达，而不只是没有 HTTP 请求。
    assert!(matches!(listener.accept(), Err(error) if error.kind() == ErrorKind::WouldBlock));
}

#[test]
fn valid_broker_json_larger_than_ten_mib_is_accepted() {
    let fixture = Fixture::new();
    let (server, endpoint) = broker();
    let body = serde_json::to_vec(&json!({
        "channels": [],
        "padding": "x".repeat(10 * 1024 * 1024 + 1024)
    }))
    .unwrap();
    let output = run(fixture.command(Some(&endpoint)), Some((&server, 200, body)));
    assert_ready(&output);
}
