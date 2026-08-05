use crate::git_remote_registry::{self, GitRemoteRegistry};
use crate::repowise_i18n_proxy::RepowiseI18nProxy;
use serde_json::Value;
use std::io::Read;
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use tiny_http::{Response, Server};

pub fn start_proxy(upstream_port: u16, proxy_port: u16, lang: &str) -> ! {
    let proxy = Arc::new(RepowiseI18nProxy::new());
    let upstream_url = Arc::new(format!("http://localhost:{}", upstream_port));
    let lang = Arc::new(lang.to_string());

    let server = Arc::new(
        Server::http(format!("127.0.0.1:{}", proxy_port)).expect("Failed to start proxy server"),
    );
    eprintln!(
        "Repowise proxy started on port {}. Forwarding to {}",
        proxy_port, upstream_url
    );
    eprintln!("Language: {}", lang);

    spawn_git_remote_warmup(Arc::clone(&upstream_url));

    loop {
        let request = match server.recv() {
            Ok(request) => request,
            Err(e) => {
                eprintln!("Proxy accept error: {}", e);
                continue;
            }
        };

        let proxy = Arc::clone(&proxy);
        let upstream_url = Arc::clone(&upstream_url);
        let lang = Arc::clone(&lang);

        thread::spawn(move || {
            handle_request(request, &proxy, &upstream_url, &lang);
        });
    }
}

/// Phase 2 of the GitHub/GitLab/Gitea remote-linkage design
/// (`docs/github-gitlab-remote-linkage-design.md`, issue #191): warms the
/// remote-link sidecar once at proxy startup, in its own background thread
/// so it never delays the first proxied request. `repowise serve` and this
/// proxy are started as separate processes with no ordering guarantee, so
/// this retries a few times before giving up -- a cache that's merely late
/// is fine (Phase 3's on-demand `get_or_resolve` path fills gaps lazily);
/// there is no HTTP surface serving this data yet.
fn spawn_git_remote_warmup(upstream_url: Arc<String>) {
    thread::spawn(move || {
        let overrides_path = git_remote_registry::discover_git_host_overrides(None);
        let overrides = overrides_path
            .map(|p| git_remote_registry::load_git_host_overrides(&p))
            .unwrap_or_default();

        let mut results = Vec::new();
        for attempt in 0..5 {
            results = git_remote_registry::warm_up_from_upstream(&upstream_url, &overrides);
            if !results.is_empty() {
                break;
            }
            if attempt < 4 {
                thread::sleep(Duration::from_secs(1));
            }
        }

        if results.is_empty() {
            eprintln!(
                "git-remote-registry: warm-up found 0 repos after retries (upstream not ready, or /api/repos unreachable/empty)"
            );
            return;
        }

        let mut registry = GitRemoteRegistry::load(git_remote_registry::default_registry_path());
        for (local_path, info) in &results {
            registry.upsert(local_path, info.clone());
        }
        match registry.save() {
            Ok(()) => eprintln!(
                "git-remote-registry: warmed {} repo(s) into {}",
                results.len(),
                git_remote_registry::default_registry_path().display()
            ),
            Err(e) => eprintln!("git-remote-registry: failed to save registry: {}", e),
        }
    });
}

fn handle_request(
    request: tiny_http::Request,
    proxy: &RepowiseI18nProxy,
    upstream_url: &str,
    lang: &str,
) {
    let path = request.url().to_string();

    // Synthetic route (design doc §6.2), short-circuited before the
    // upstream forward: a reverse proxy owns its own routes rather than
    // trying to make repowise aware of them. Served straight from the
    // sidecar cache Phase 2 warms, with no upstream round-trip at all.
    if path == git_remote_registry::REMOTE_LINKS_ROUTE {
        let registry = GitRemoteRegistry::load(git_remote_registry::default_registry_path());
        let body = registry.to_remote_links_json().to_string();
        let resp = Response::from_string(body).with_header(
            tiny_http::Header::from_bytes(&b"Content-Type"[..], b"application/json".as_slice())
                .unwrap(),
        );
        let _ = request.respond(resp);
        return;
    }

    let upstream_uri = format!("{}{}", upstream_url, path);

    match ureq::get(&upstream_uri).call() {
        Ok(response) => {
            let content_type = response.content_type().to_string();
            let is_text = content_type.contains("application/json")
                || content_type.contains("text/html");

            if is_text {
                let mut body = String::new();
                let _ = response.into_reader().read_to_string(&mut body);

                let translated_body = if content_type.contains("application/json") {
                    if let Ok(json) = serde_json::from_str::<Value>(&body) {
                        proxy.translate_response(lang, &json).to_string()
                    } else {
                        body
                    }
                } else {
                    let translated = proxy.translate_html(lang, &body);
                    inject_before_body_close(
                        &translated,
                        &git_remote_registry::build_remote_link_injection_script(),
                    )
                };

                let resp = Response::from_string(translated_body).with_header(
                    tiny_http::Header::from_bytes(&b"Content-Type"[..], content_type.as_bytes())
                        .unwrap(),
                );
                let _ = request.respond(resp);
            } else {
                let mut bytes = Vec::new();
                let _ = response.into_reader().read_to_end(&mut bytes);
                let resp = Response::from_data(bytes).with_header(
                    tiny_http::Header::from_bytes(&b"Content-Type"[..], content_type.as_bytes())
                        .unwrap(),
                );
                let _ = request.respond(resp);
            }
        }
        Err(e) => {
            eprintln!("Upstream error for {}: {}", path, e);
            let _ = request.respond(Response::from_string("Proxy error").with_status_code(502));
        }
    }
}

/// Appends a script block before `</body>`, or at the end if the HTML has
/// no closing body tag. `repowise_i18n_proxy::translate_html` does the same
/// thing for its own script; this is the second, independent injection
/// (design doc §6.2) for the remote-link script, kept here rather than
/// added to `repowise_i18n_proxy.rs` so that file stays solely about
/// translation.
fn inject_before_body_close(html: &str, script: &str) -> String {
    if let Some(pos) = html.rfind("</body>") {
        let mut result = String::with_capacity(html.len() + script.len());
        result.push_str(&html[..pos]);
        result.push_str(script);
        result.push_str(&html[pos..]);
        result
    } else {
        format!("{}{}", html, script)
    }
}
