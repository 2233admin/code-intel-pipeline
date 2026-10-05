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

/// Response headers safe to relay verbatim from the upstream HTTP response
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

    let result = request_upstream(request.headers(), &method, upstream_uri, &request_body);

    // Keep error statuses as real responses with their bodies. Only a failure
    // without a usable upstream response becomes this proxy's synthetic 502.
    match result {
        Ok(response) => {
            forward_response(request, response.status().as_u16(), response, proxy, lang)
        }
        Err(e) => {
            eprintln!("Upstream error for {} {}: {}", method, path, e);
            let _ = request.respond(Response::from_string("Proxy error").with_status_code(502));
        }
    }
}

/// Follow the old redirect policy without losing unfollowable real responses.
/// Every followed redirect drops the body; POST/PUT/DELETE 307/308 are relayed.
fn request_upstream(
    headers: &[tiny_http::Header],
    mut method: &str,
    uri: String,
    mut body: &[u8],
) -> Result<ureq::http::Response<ureq::Body>, ureq::Error> {
    let client = ureq::Agent::config_builder()
        .proxy(None)
        .max_redirects(0)
        .max_redirects_will_error(false)
        .timeout_connect(Some(Duration::from_secs(30)))
        .http_status_as_error(false)
        .allow_non_standard_methods(true)
        .build()
        .new_agent();
    let mut uri =
        ureq::http::Uri::try_from(uri).map_err(|error| ureq::Error::BadUri(error.to_string()))?;
    let mut completed = 0;
    loop {
        let mut req = ureq::http::Request::builder()
            .method(method)
            .uri(uri.clone());
        for header in headers {
            let name = header.field.as_str().as_str();
            if !is_forwardable_request_header(name) {
                continue;
            }
            // ureq 2 replaced non-X headers but appended repeated X headers.
            if !name.starts_with("X-") && !name.starts_with("x-") {
                if let Some(headers) = req.headers_mut() {
                    headers.remove(name);
                }
            }
            req = req.header(name, header.value.as_str());
        }
        let response = if body.is_empty() {
            client.run(req.body(())?)
        } else {
            client.run(req.body(body)?)
        }?;
        let status = response.status().as_u16();
        if !(300..399).contains(&status) {
            return Ok(response);
        }
        if completed == 4 {
            return Err(ureq::Error::TooManyRedirects);
        }
        let Some(location) = response.headers().get("location") else {
            return Ok(response);
        };
        let location = std::str::from_utf8(location.as_bytes())
            .map_err(|error| ureq::Error::BadUri(error.to_string()))?;
        let mut target = url::Url::parse(&uri.to_string())
            .and_then(|base| base.join(location))
            .map_err(|error| ureq::Error::BadUri(error.to_string()))?;
        method = match status {
            301 | 302 | 303 if method == "HEAD" => "HEAD",
            301 | 302 | 303 => "GET",
            307 | 308 if matches!(method, "GET" | "HEAD" | "OPTIONS" | "TRACE") => method,
            _ => return Ok(response),
        };
        target.set_fragment(None);
        uri = ureq::http::Uri::try_from(String::from(target))
            .map_err(|error| ureq::Error::BadUri(error.to_string()))?;
        body = &[];
        completed += 1;
    }
}

/// Builds a `Content-Type` header from an upstream-supplied value, falling
/// back to a safe default instead of panicking when the upstream sends
/// bytes `tiny_http::Header` rejects (non-ASCII, stray control chars).
fn content_type_header(content_type: &str) -> tiny_http::Header {
    tiny_http::Header::from_bytes(&b"Content-Type"[..], content_type.as_bytes()).unwrap_or_else(
        |_| {
            tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"application/octet-stream"[..])
                .expect("static content-type header is valid")
        },
    )
}

/// Relays every real upstream HTTP response through `tiny_http`, applying
/// the i18n/remote-link translation passes for text bodies along the way.
fn forward_response(
    request: tiny_http::Request,
    status: u16,
    response: ureq::http::Response<ureq::Body>,
    proxy: &RepowiseI18nProxy,
    lang: &str,
) {
    let (parts, upstream_body) = response.into_parts();
    let content_type = parts
        .headers
        .get("content-type")
        .and_then(|value| std::str::from_utf8(value.as_bytes()).ok())
        .unwrap_or("text/plain")
        .split(';')
        .next()
        .unwrap_or("text/plain");
    let is_text = content_type.contains("application/json") || content_type.contains("text/html");

    let mut resp = if is_text {
        let mut body = String::new();
        let _ = upstream_body.into_reader().read_to_string(&mut body);

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

        Response::from_string(translated_body)
    } else {
        let mut bytes = Vec::new();
        let _ = upstream_body.into_reader().read_to_end(&mut bytes);
        Response::from_data(bytes)
    }
    .with_status_code(status)
    .with_header(content_type_header(content_type));

    // Relay every value directly, preserving repeated Set-Cookie entries.
    for (name, value) in &parts.headers {
        if !is_forwardable_response_header(name.as_str()) {
            continue;
        }
        if let Ok(value) = std::str::from_utf8(value.as_bytes()) {
            if let Ok(header) =
                tiny_http::Header::from_bytes(name.as_str().as_bytes(), value.as_bytes())
            {
                resp = resp.with_header(header);
            }
        }
    }
    let _ = request.respond(resp);
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
