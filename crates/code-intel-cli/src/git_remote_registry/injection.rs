use serde_json::json;

/// The proxy-served route path for [`super::GitRemoteRegistry::to_remote_links_json`]
/// (design doc §6.2). Namespaced under `/__code-intel/` so it can never
/// collide with a real repowise API route as repowise evolves.
pub const REMOTE_LINKS_ROUTE: &str = "/__code-intel/remote-links.json";

/// Client-side injection script (design doc §6.2): fetches
/// [`REMOTE_LINKS_ROUTE`] once, then decorates every repo-card link
/// (`/repos/{id}/overview`, confirmed by inspecting the live DOM) with a
/// small "open on GitHub/GitLab/Gitea" anchor.
///
/// Deliberately does *not* reuse `repowise_i18n_proxy`'s MutationObserver --
/// this is a separate concern with a separate dictionary (an id->link map
/// instead of a string->string one), and keeping them independent matches
/// this codebase's existing file-per-plugin shape rather than coupling two
/// plugins that don't need to share state.
///
/// Idempotency is content-based, not marker-based, for the same reason
/// `repowise_i18n_proxy`'s text-node translation is: repowise's repo list
/// is a large virtualized list, so React can recycle a card's DOM node for
/// a different repo on scroll. A one-time "already decorated" flag on the
/// node would then wrongly keep a stale link attached to a recycled card.
/// Instead, every pass compares the *current* desired href against what's
/// already appended and only touches the DOM when they differ.
pub fn build_remote_link_injection_script() -> String {
    format!(
        r#"<script id="code-intel-remote-links-injected">
(function() {{
  fetch({route}).then(function(r) {{ return r.ok ? r.json() : {{}}; }}).then(function(map) {{
    if (!map || Object.keys(map).length === 0) return;
    var ID_RE = /\/repos\/([0-9a-f]{{32}})\/overview/;

    function decorate(a) {{
      var href = a.getAttribute && a.getAttribute('href');
      var existing = a.querySelector(':scope > .ci-remote-link');
      var m = href && href.match(ID_RE);
      var info = m && map[m[1]];
      var desired = info && info.web_base_url;
      // Second layer, redundant with the server-side scheme allowlist in
      // resolve_remote: never assign an href whose scheme isn't http/https,
      // so a payload that somehow bypassed the server check still can't
      // become a clickable javascript: link here.
      if (desired && !/^https?:\/\//i.test(desired)) desired = null;
      if (!desired) {{
        if (existing) existing.remove();
        return;
      }}
      if (existing && existing.getAttribute('href') === desired) return;
      if (existing) existing.remove();
      var link = document.createElement('a');
      link.className = 'ci-remote-link';
      link.href = desired;
      link.target = '_blank';
      link.rel = 'noopener noreferrer';
      var label = info.host_type === 'github' ? 'GitHub'
        : info.host_type === 'gitlab' ? 'GitLab'
        : info.host_type === 'gitea' ? 'Gitea'
        : 'remote';
      link.title = 'Open on ' + label;
      link.textContent = ' ↗';
      link.style.marginLeft = '4px';
      link.style.opacity = '0.6';
      link.addEventListener('click', function(ev) {{ ev.stopPropagation(); }});
      a.appendChild(link);
    }}

    function scan(root) {{
      if (!root || root.nodeType !== Node.ELEMENT_NODE) return;
      if (root.tagName === 'A') decorate(root);
      if (!root.querySelectorAll) return;
      var links = root.querySelectorAll('a[href*="/repos/"]');
      for (var i = 0; i < links.length; i++) decorate(links[i]);
    }}

    scan(document.body);
    var observer = new MutationObserver(function(mutations) {{
      for (var i = 0; i < mutations.length; i++) {{
        var m = mutations[i];
        if (m.type === 'characterData') {{
          scan(m.target.parentElement);
        }} else {{
          for (var j = 0; j < m.addedNodes.length; j++) scan(m.addedNodes[j]);
        }}
      }}
    }});
    observer.observe(document.body, {{ childList: true, subtree: true, characterData: true }});
  }}).catch(function() {{}});
}})();
</script>"#,
        route = json!(REMOTE_LINKS_ROUTE)
    )
}
