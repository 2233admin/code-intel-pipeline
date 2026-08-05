use crate::git_remote_registry::{self, GitRemoteRegistry};
use crate::repowise_i18n_proxy::RepowiseI18nProxy;
use serde_json::Value;
use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use tiny_http::{Response, Server};

pub fn start_proxy(upstream_port: u16, proxy_port: u16, lang: &str) -> ! {
    let proxy = Arc::new(RepowiseI18nProxy::new());
    let upstream_url = Arc::new(format!("http://localhost:{}", upstream_port));
    let lang = Arc::new(lang.to_string());
    let warmup_in_flight = Arc::new(AtomicBool::new(false));

    let server = Arc::new(
        Server::http(format!("127.0.0.1:{}", proxy_port)).expect("Failed to start proxy server"),
    );
    eprintln!(
        "Repowise proxy started on port {}. Forwarding to {}",
        proxy_port, upstream_url
    );
    eprintln!("Language: {}", lang);

    spawn_git_remote_warmup(Arc::clone(&upstream_url), Arc::clone(&warmup_in_flight));

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
        let warmup_in_flight = Arc::clone(&warmup_in_flight);

        thread::spawn(move || {
            handle_request(request, &proxy, &upstream_url, &lang, &warmup_in_flight);
        });
    }
}

/// Startup-to-first-success window for [`spawn_git_remote_warmup`]'s retry
/// loop: 1+2+4+8+16+32+60+60s ≈ 3 minutes total. `repowise` indexes the
/// whole workspace (135 repos in this environment) before `/api/repos`
/// starts answering, and a fixed 5×1s window (the original attempt here)
/// gave up long before that finished on a real workspace, leaving the
/// registry empty for the rest of the process's life.
const WARMUP_BACKOFF_SECS: [u64; 8] = [1, 2, 4, 8, 16, 32, 60, 60];

/// Phase 2 of the GitHub/GitLab/Gitea remote-linkage design
/// (`docs/github-gitlab-remote-linkage-design.md`, issue #191): warms the
/// remote-link sidecar once at proxy startup, in its own background thread
/// so it never delays the first proxied request. `repowise serve` and this
/// proxy are started as separate processes with no ordering guarantee, so
/// this retries with backoff before giving up -- a cache that's merely late
/// is fine ([`GitRemoteRegistry::get_or_resolve`] fills gaps lazily once
/// Phase 3 wires it to the route), but giving up too early leaves the
/// registry empty forever, since nothing else re-triggers this pass.
fn spawn_git_remote_warmup(upstream_url: Arc<String>, warmup_in_flight: Arc<AtomicBool>) {
    // Compare-and-swap dedup: if a warm-up is already running (startup pass,
    // or a prior self-heal retry from `handle_request`), don't stack a
    // second one on top of it -- both would hit `/api/repos` and shell out
    // `git remote get-url origin` per repo concurrently for no benefit.
    if warmup_in_flight.swap(true, Ordering::SeqCst) {
        return;
    }

    thread::spawn(move || {
        let overrides_path = git_remote_registry::discover_git_host_overrides(None);
        let overrides = overrides_path
            .map(|p| git_remote_registry::load_git_host_overrides(&p))
            .unwrap_or_default();

        let mut results = Vec::new();
        for (attempt, wait_secs) in WARMUP_BACKOFF_SECS.iter().enumerate() {
            results = git_remote_registry::warm_up_from_upstream(&upstream_url, &overrides);
            if !results.is_empty() {
                break;
            }
            if attempt < WARMUP_BACKOFF_SECS.len() - 1 {
                thread::sleep(Duration::from_secs(*wait_secs));
            }
        }

        if results.is_empty() {
            eprintln!(
                "git-remote-registry: warm-up found 0 repos after retries (upstream not ready, or /api/repos unreachable/empty)"
            );
            warmup_in_flight.store(false, Ordering::SeqCst);
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
        warmup_in_flight.store(false, Ordering::SeqCst);
    });
}

/// Request headers safe to forward verbatim to the upstream `ureq` request.
/// Two exclusion reasons are folded into one predicate:
///
/// - Hop-by-hop / connection-specific headers (`Host`, `Content-Length`,
///   `Connection`, `Transfer-Encoding`): forwarding these verbatim would
///   either be meaningless to a new connection (`Host` is re-derived from
///   `upstream_uri`) or wrong once ureq recomputes them for the outgoing
///   request it actually sends (`Content-Length`, `Transfer-Encoding`).
/// - Credential headers (`Authorization`, `Cookie`): the upstream listener
///   (`repowise serve` on `http://localhost:<upstream_port>`) is an
///   unauthenticated local process, and `localhost` alone doesn't protect
///   these credentials from another local process bound to that port. This
///   proxy has no documented need to pass a client's own credentials
///   through to it, so they're stripped rather than relayed (CodeRabbit
///   review, PR #194, comment 3718037783).
fn is_forwardable_request_header(name: &str) -> bool {
    !matches!(
        name.to_ascii_lowercase().as_str(),
        "host" | "content-length" | "connection" | "transfer-encoding" | "authorization" | "cookie"
    )
}

/// Response headers safe to relay verbatim from the upstream `ureq::Response`
/// back to the client via `tiny_http`. Two exclusion reasons:
///
/// - The full RFC 7230 §6.1 hop-by-hop set (`Connection`, `Keep-Alive`,
///   `Proxy-Authenticate`, `Proxy-Authorization`, `TE`, `Trailer`,
///   `Transfer-Encoding`, `Upgrade`): meaningful only for the single
///   upstream<->proxy hop, not for the proxy<->client hop this response is
///   headed to.
/// - Headers this proxy would otherwise misrepresent, since it may re-encode
///   or translate the body before resending it (`Content-Length`,
///   `Content-Encoding`). `Content-Type` is excluded too -- `forward_response`
///   already sets it separately from the (possibly re-detected) content type.
///
/// Everything else -- `Set-Cookie`, `WWW-Authenticate`, `Cache-Control`, etc.
/// -- passes through unchanged (CodeRabbit review, PR #194, comment
/// 3718037787).
fn is_forwardable_response_header(name: &str) -> bool {
    !matches!(
        name.to_ascii_lowercase().as_str(),
        "content-type"
            | "content-length"
            | "content-encoding"
            | "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
    )
}

fn handle_request(
    mut request: tiny_http::Request,
    proxy: &RepowiseI18nProxy,
    upstream_url: &str,
    lang: &str,
    warmup_in_flight: &Arc<AtomicBool>,
) {
    let path = request.url().to_string();

    // Synthetic route (design doc §6.2), short-circuited before the
    // upstream forward: a reverse proxy owns its own routes rather than
    // trying to make repowise aware of them. Served straight from the
    // sidecar cache Phase 2 warms, with no upstream round-trip at all.
    if path == git_remote_registry::REMOTE_LINKS_ROUTE {
        let registry = GitRemoteRegistry::load(git_remote_registry::default_registry_path());
        // Self-heal: an empty registry could mean startup warm-up is still
        // retrying (fine, it'll fill in), or it already gave up (design
        // doc's original 4s window did this on a real 135-repo workspace).
        // Kick a retry rather than serving `{}` for the rest of the
        // process's life; `spawn_git_remote_warmup`'s own dedup means this
        // is a no-op if one's already running.
        if registry.len() == 0 {
            spawn_git_remote_warmup(
                Arc::new(upstream_url.to_string()),
                Arc::clone(warmup_in_flight),
            );
        }
        let body = registry.to_remote_links_json().to_string();
        let resp = Response::from_string(body).with_header(
            tiny_http::Header::from_bytes(&b"Content-Type"[..], b"application/json".as_slice())
                .unwrap(),
        );
        let _ = request.respond(resp);
        return;
    }

    let upstream_uri = format!("{}{}", upstream_url, path);
    let method = request.method().to_string();

    // Read the body before building the outgoing request: tiny_http gives
    // access to it through `&mut Request`, and once `request` is consumed by
    // `.respond()` at the end of this function it's gone. Reading it
    // unconditionally (even for GET, where it's just empty) keeps this one
    // code path instead of a per-method special case.
    let mut request_body = Vec::new();
    let _ = request.as_reader().read_to_end(&mut request_body);

    let mut req = ureq::request(&method, &upstream_uri);
    for header in request.headers() {
        let name = header.field.as_str().as_str();
        if !is_forwardable_request_header(name) {
            continue;
        }
        req = req.set(name, header.value.as_str());
    }

    let result = if request_body.is_empty() {
        req.call()
    } else {
        req.send_bytes(&request_body)
    };

    // ureq treats any 4xx/5xx as `Err(Error::Status(code, response))`, but
    // that's still a real upstream response with a real body worth
    // forwarding (an error page, a JSON error payload) -- only
    // `Error::Transport` (connection refused, timeout, DNS failure) means
    // there's genuinely no response to relay, which is the only case that
    // becomes this proxy's own synthetic 502.
    match result {
        Ok(response) => forward_response(request, response.status(), response, proxy, lang),
        Err(ureq::Error::Status(code, response)) => {
            forward_response(request, code, response, proxy, lang)
        }
        Err(e) => {
            eprintln!("Upstream error for {} {}: {}", method, path, e);
            let _ = request.respond(Response::from_string("Proxy error").with_status_code(502));
        }
    }
}

/// Relays one upstream `ureq::Response` back through `tiny_http`, applying
/// the i18n/remote-link translation passes for text bodies along the way.
/// Shared between the success path and the `Error::Status` path in
/// `handle_request` -- both hand it a real response with a real status
/// code, just reached via different `Result` arms.
fn forward_response(
    request: tiny_http::Request,
    status: u16,
    response: ureq::Response,
    proxy: &RepowiseI18nProxy,
    lang: &str,
) {
    let content_type = response.content_type().to_string();
    let is_text = content_type.contains("application/json") || content_type.contains("text/html");

    // Collect the safe end-to-end headers before `response` is consumed by
    // `.into_reader()` below. `headers_names()` can repeat a name when the
    // upstream sent it multiple times (e.g. `Set-Cookie`), so dedup by name
    // and pull every value for it via `.all()` rather than just the first.
    let mut seen_header_names = std::collections::HashSet::new();
    let mut extra_headers = Vec::new();
    for name in response.headers_names() {
        if !is_forwardable_response_header(&name) || !seen_header_names.insert(name.clone()) {
            continue;
        }
        for value in response.all(&name) {
            if let Ok(header) = tiny_http::Header::from_bytes(name.as_bytes(), value.as_bytes()) {
                extra_headers.push(header);
            }
        }
    }

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

        let mut resp = Response::from_string(translated_body)
            .with_status_code(status)
            .with_header(
                tiny_http::Header::from_bytes(&b"Content-Type"[..], content_type.as_bytes())
                    .unwrap(),
            );
        for header in extra_headers {
            resp = resp.with_header(header);
        }
        let _ = request.respond(resp);
    } else {
        let mut bytes = Vec::new();
        let _ = response.into_reader().read_to_end(&mut bytes);
        let mut resp = Response::from_data(bytes)
            .with_status_code(status)
            .with_header(
                tiny_http::Header::from_bytes(&b"Content-Type"[..], content_type.as_bytes())
                    .unwrap(),
            );
        for header in extra_headers {
            resp = resp.with_header(header);
        }
        let _ = request.respond(resp);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_headers_strip_credentials() {
        // CodeRabbit PR #194 comment 3718037783: the upstream `repowise
        // serve` listener is unauthenticated, so a client's own credentials
        // must never be forwarded to it.
        assert!(!is_forwardable_request_header("Authorization"));
        assert!(!is_forwardable_request_header("authorization"));
        assert!(!is_forwardable_request_header("Cookie"));
        assert!(!is_forwardable_request_header("cookie"));
    }

    #[test]
    fn request_headers_strip_hop_by_hop_and_connection_specific() {
        assert!(!is_forwardable_request_header("Host"));
        assert!(!is_forwardable_request_header("Content-Length"));
        assert!(!is_forwardable_request_header("Connection"));
        assert!(!is_forwardable_request_header("Transfer-Encoding"));
    }

    #[test]
    fn request_headers_pass_through_everything_else() {
        assert!(is_forwardable_request_header("Accept"));
        assert!(is_forwardable_request_header("X-Requested-With"));
        assert!(is_forwardable_request_header("Content-Type"));
    }

    #[test]
    fn response_headers_strip_body_shape_and_hop_by_hop() {
        // These describe the upstream body's exact framing/encoding, which
        // `forward_response` may have already invalidated by translating or
        // re-serializing the body before resending it.
        assert!(!is_forwardable_response_header("Content-Length"));
        assert!(!is_forwardable_response_header("Content-Encoding"));
        // Set separately by `forward_response` already -- forwarding it
        // again here would risk a duplicate/conflicting header.
        assert!(!is_forwardable_response_header("Content-Type"));
    }

    #[test]
    fn response_headers_strip_full_hop_by_hop_set() {
        // RFC 7230 §6.1's full hop-by-hop category, not just the subset
        // CodeRabbit named literally -- these are meaningful only for the
        // upstream<->proxy hop and must not leak onto the proxy<->client one.
        for name in [
            "Connection",
            "Keep-Alive",
            "Proxy-Authenticate",
            "Proxy-Authorization",
            "TE",
            "Trailer",
            "Transfer-Encoding",
            "Upgrade",
        ] {
            assert!(
                !is_forwardable_response_header(name),
                "{name} should be excluded as hop-by-hop"
            );
        }
    }

    #[test]
    fn response_headers_relay_everything_else() {
        // CodeRabbit PR #194 comment 3718037787: these were previously
        // silently dropped and must now be relayed to the client.
        assert!(is_forwardable_response_header("Set-Cookie"));
        assert!(is_forwardable_response_header("WWW-Authenticate"));
        assert!(is_forwardable_response_header("Cache-Control"));
        assert!(is_forwardable_response_header("ETag"));
    }
}
