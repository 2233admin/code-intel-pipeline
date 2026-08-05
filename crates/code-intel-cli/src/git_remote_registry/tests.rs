use super::*;

fn overrides_with_gitea() -> HashMap<String, HostOverride> {
    let mut m = HashMap::new();
    m.insert(
        "git.xart.top:8418".to_string(),
        HostOverride {
            host_type: HostType::Gitea,
            web_base_url: "https://git.xart.top:8418".to_string(),
        },
    );
    m
}

#[test]
fn github_with_git_suffix() {
    let info = resolve_remote(
        Some("https://github.com/2233admin/code-intel-pipeline.git"),
        &HashMap::new(),
    );
    assert_eq!(info.host_type, HostType::Github);
    assert_eq!(info.owner.as_deref(), Some("2233admin"));
    assert_eq!(info.repo.as_deref(), Some("code-intel-pipeline"));
    assert_eq!(
        info.web_base_url.as_deref(),
        Some("https://github.com/2233admin/code-intel-pipeline")
    );
    assert!(!info.has_credentials_stripped);
    assert!(!info.is_plaintext_transport);
}

#[test]
fn github_without_git_suffix() {
    let info = resolve_remote(
        Some("https://github.com/TraderAlice/OpenAlice-latest"),
        &HashMap::new(),
    );
    assert_eq!(info.host_type, HostType::Github);
    assert_eq!(info.owner.as_deref(), Some("TraderAlice"));
    assert_eq!(info.repo.as_deref(), Some("OpenAlice-latest"));
}

#[test]
fn gitea_custom_port_with_embedded_credentials_is_stripped() {
    let raw = "https://someuser:ghp_faketokenvaluexxxx@git.xart.top:8418/owner/katana-kernel.git";
    let info = resolve_remote(Some(raw), &overrides_with_gitea());
    assert!(info.has_credentials_stripped);
    assert_eq!(info.host_type, HostType::Gitea);
    assert_eq!(
        info.web_base_url.as_deref(),
        Some("https://git.xart.top:8418/owner/katana-kernel")
    );
    // The whole point: no code path may retain the credential substring.
    let normalized = info.remote_url_normalized.unwrap();
    assert!(!normalized.contains("someuser"));
    assert!(!normalized.contains("ghp_faketokenvaluexxxx"));
    assert!(!normalized.contains('@'));
}

#[test]
fn gitea_http_no_git_suffix_flags_plaintext_transport() {
    let info = resolve_remote(
        Some("http://git.xart.top:8418/owner/red-queen"),
        &overrides_with_gitea(),
    );
    assert!(info.is_plaintext_transport);
    assert_eq!(info.repo.as_deref(), Some("red-queen"));
    assert_eq!(info.host_type, HostType::Gitea);
}

#[test]
fn gitea_without_override_falls_back_to_generic() {
    let info = resolve_remote(
        Some("https://git.xart.top:8418/owner/katana-kernel.git"),
        &HashMap::new(),
    );
    assert_eq!(info.host_type, HostType::Generic);
    // Still linkable to the repo root, per design doc §5.3.
    assert_eq!(
        info.web_base_url.as_deref(),
        Some("https://git.xart.top:8418/owner/katana-kernel")
    );
}

#[test]
fn ssh_scp_style_remote() {
    let info = resolve_remote(
        Some("git@github.com:2233admin/code-intel-pipeline.git"),
        &HashMap::new(),
    );
    assert_eq!(info.host_type, HostType::Github);
    assert_eq!(info.owner.as_deref(), Some("2233admin"));
    assert_eq!(info.repo.as_deref(), Some("code-intel-pipeline"));
}

#[test]
fn no_origin_configured() {
    let info = resolve_remote(None, &HashMap::new());
    assert_eq!(info.host_type, HostType::None);
    assert!(info.remote_url_normalized.is_none());
}

#[test]
fn empty_origin_string_treated_as_none() {
    let info = resolve_remote(Some(""), &HashMap::new());
    assert_eq!(info.host_type, HostType::None);
}

#[test]
fn name_alias_divergence_does_not_affect_resolution() {
    // Design doc §7.2: workspace alias `workbench-kernel` vs remote repo
    // name `katana-kernel`. resolve_remote only ever sees the remote
    // URL, never the alias, so this is really a caller-discipline note
    // -- asserted here so a future refactor can't accidentally start
    // passing the alias in instead of the real remote URL.
    let info = resolve_remote(
        Some("https://github.com/owner/katana-kernel.git"),
        &HashMap::new(),
    );
    assert_eq!(info.repo.as_deref(), Some("katana-kernel"));
}

#[test]
fn load_git_host_overrides_skips_malformed_entries() {
    let dir = std::env::temp_dir().join(format!(
        "code-intel-git-host-overrides-test-{}",
        std::process::id()
    ));
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join("git-hosts.json");
    fs::write(
        &path,
        r#"{
            "git.xart.top:8418": {"type": "gitea", "web_base_url": "https://git.xart.top:8418"},
            "broken.example.com": {"type": "gitea"}
        }"#,
    )
    .unwrap();

    let loaded = load_git_host_overrides(&path);
    assert_eq!(loaded.len(), 1);
    assert!(loaded.contains_key("git.xart.top:8418"));

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn registry_roundtrips_through_disk() {
    let dir = std::env::temp_dir().join(format!(
        "code-intel-git-remote-registry-test-{}",
        std::process::id()
    ));
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join("registry.json");

    let mut registry = GitRemoteRegistry::load(path.clone());
    let info = resolve_remote(
        Some("https://github.com/2233admin/code-intel-pipeline.git"),
        &HashMap::new(),
    );
    registry.upsert(r"D:\projects\code-intel-pipeline", info);
    registry.save().unwrap();

    let reloaded = GitRemoteRegistry::load(path);
    let entry = reloaded
        .get(r"D:\Projects\Code-Intel-Pipeline")
        .expect("case-insensitive lookup should hit");
    assert_eq!(entry.host_type, HostType::Github);
    assert_eq!(entry.owner.as_deref(), Some("2233admin"));

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn git_remote_origin_reads_this_repos_real_remote() {
    // cargo test's cwd is the crate dir, which is inside this actual
    // git repo -- exercises the real shell-out against real git state
    // instead of only synthetic URL strings.
    let cwd = std::env::current_dir().unwrap();
    let raw = git_remote_origin(cwd.to_str().unwrap());
    match raw {
        Some(url) => assert!(url.contains("code-intel-pipeline")),
        None => {
            // Legitimate in a checkout with no configured origin (design
            // doc §3 corpus includes 21 such repos) -- not a test failure.
        }
    }
}

#[test]
fn git_remote_origin_returns_none_for_non_git_directory() {
    let dir = std::env::temp_dir().join(format!(
        "code-intel-not-a-git-repo-test-{}",
        std::process::id()
    ));
    fs::create_dir_all(&dir).unwrap();
    assert!(git_remote_origin(dir.to_str().unwrap()).is_none());
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn get_or_resolve_caches_after_first_call() {
    let dir = std::env::temp_dir().join(format!(
        "code-intel-get-or-resolve-test-{}",
        std::process::id()
    ));
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join("registry.json");
    let mut registry = GitRemoteRegistry::load(path);

    let cwd = std::env::current_dir().unwrap();
    let cwd_str = cwd.to_str().unwrap();
    let first = registry.get_or_resolve(cwd_str, &HashMap::new()).clone();
    assert_eq!(registry.len(), 1);

    // Second call must hit the cache, not shell out again -- assert by
    // checking the entry is unchanged and no new entries were added.
    let second = registry.get_or_resolve(cwd_str, &HashMap::new());
    assert_eq!(first.host_type, second.host_type);
    assert_eq!(registry.len(), 1);

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn remote_links_json_keys_by_repo_id_and_omits_unlinkable() {
    let dir = std::env::temp_dir().join(format!(
        "code-intel-remote-links-json-test-{}",
        std::process::id()
    ));
    fs::create_dir_all(&dir).unwrap();
    let mut registry = GitRemoteRegistry::load(dir.join("registry.json"));

    let mut linkable = resolve_remote(
        Some("https://github.com/2233admin/code-intel-pipeline.git"),
        &HashMap::new(),
    );
    linkable.repo_id = Some("4d311154870945d488716522d1913dba".to_string());
    registry.upsert(r"D:\projects\code-intel-pipeline", linkable);

    // No origin -> host_type none -> no web_base_url -> must be omitted,
    // even though it has a repo_id.
    let mut unlinkable = resolve_remote(None, &HashMap::new());
    unlinkable.repo_id = Some("00000000000000000000000000000000".to_string());
    registry.upsert(r"D:\projects\DaHuaCyou_run", unlinkable);

    let json = registry.to_remote_links_json();
    let obj = json.as_object().unwrap();
    assert_eq!(obj.len(), 1);
    let entry = &obj["4d311154870945d488716522d1913dba"];
    assert_eq!(
        entry["web_base_url"],
        "https://github.com/2233admin/code-intel-pipeline"
    );
    assert_eq!(entry["host_type"], "github");
    assert!(!obj.contains_key("00000000000000000000000000000000"));

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn discover_git_host_overrides_prefers_explicit_path() {
    let dir = std::env::temp_dir().join(format!(
        "code-intel-discover-overrides-test-{}",
        std::process::id()
    ));
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join("git-hosts.json");
    fs::write(&path, "{}").unwrap();

    let found = discover_git_host_overrides(Some(&path));
    assert_eq!(found.as_deref(), Some(path.as_path()));

    fs::remove_dir_all(&dir).ok();
}
