//! The agent assistance catalog is the only place a candidate's fit, license,
//! security, integration, and reversibility rating is decided, and it is the
//! only input `assistance.discovery` resolves candidates from. These tests
//! keep it consumable by that capability and honest about what it claims:
//! a catalog that the discovery core would reject, or that routes to an
//! entrypoint it never declared, is a routing table that fails at the moment
//! an operator relies on it.

#[path = "../src/assistance_discovery.rs"]
mod assistance_discovery;

use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

const CATALOG_PATH: &str = "orchestration/agent-assistance-catalog.v1.json";

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn read_json(relative: &str) -> Value {
    serde_json::from_slice(&fs::read(root().join(relative)).expect("read document"))
        .expect("document is JSON")
}

fn catalog() -> Value {
    read_json(CATALOG_PATH)
}

fn entries(catalog: &Value) -> Vec<Value> {
    catalog["entries"]
        .as_array()
        .expect("catalog entries is an array")
        .clone()
}

#[test]
fn every_catalog_candidate_survives_the_discovery_core() {
    let catalog = catalog();
    assert_eq!(catalog["schema"], "code-intel-agent-assistance-catalog.v1");
    let entries = entries(&catalog);
    assert!(!entries.is_empty(), "an empty catalog routes nowhere");
    let candidates = entries
        .iter()
        .map(|entry| entry["candidate"].clone())
        .collect::<Vec<_>>();
    let result = assistance_discovery::discover(&json!({
        "schema": "code-intel-assistance-discovery-request.v1",
        "gap": {
            "schema": "code-intel-engineering-capability-gap.v1",
            "id": "gap-catalog-coherence",
            "capability": "resolve reviewed assistance candidates",
            "description": "Every committed candidate must be resolvable by the capability that reads this catalog.",
            "constraints": ["no repository mutation"],
            "evidenceRefs": [CATALOG_PATH]
        },
        "candidates": candidates,
    }))
    .expect("every committed candidate must pass the discovery core");
    assert_eq!(
        result["dossiers"].as_array().unwrap().len(),
        entries.len(),
        "each candidate must produce exactly one dossier"
    );
    assert_eq!(result["proposalOnly"], json!(true));
    assert_eq!(result["effects"], json!([]));
}

#[test]
fn routing_only_names_entrypoints_the_entry_declares() {
    for entry in entries(&catalog()) {
        let id = entry["candidate"]["id"].as_str().expect("candidate id");
        let declared = entry["entrypoints"]
            .as_array()
            .expect("entrypoints is an array")
            .iter()
            .map(|point| point["name"].as_str().expect("entrypoint name").to_string())
            .collect::<BTreeSet<_>>();
        assert!(!declared.is_empty(), "{id} declares no entrypoint");
        for rule in entry["routing"].as_array().expect("routing is an array") {
            let target = rule["entrypoint"].as_str().expect("routing entrypoint");
            assert!(
                declared.contains(target),
                "{id} routes to {target}, which it never declares as an entrypoint"
            );
        }
    }
}

#[test]
fn install_commands_match_the_plugin_and_marketplace_they_claim() {
    let catalog = catalog();
    let marketplace = catalog["source"]["marketplace"]
        .as_str()
        .expect("source marketplace");
    let mut ids = BTreeSet::new();
    for entry in entries(&catalog) {
        let id = entry["candidate"]["id"].as_str().expect("candidate id");
        assert!(ids.insert(id.to_string()), "catalog repeats candidate {id}");
        let plugin = entry["install"]["plugin"].as_str().expect("install plugin");
        let source = entry["install"]["marketplace"]
            .as_str()
            .expect("install marketplace");
        assert_eq!(
            source, marketplace,
            "{id} installs from {source} while the catalog pins {marketplace}"
        );
        assert_eq!(
            entry["install"]["command"]
                .as_str()
                .expect("install command"),
            format!("claude plugin install {plugin}@{source}"),
            "{id} states an install command that does not match its own plugin id"
        );
        assert_eq!(
            entry["candidate"]["name"].as_str().expect("candidate name"),
            format!("{plugin}@{source}"),
            "{id} names a candidate that does not match its own plugin id"
        );
    }
}

#[test]
fn the_source_pins_a_full_commit_so_a_rating_can_be_re_checked() {
    let catalog = catalog();
    let commit = catalog["source"]["pinnedCommit"]
        .as_str()
        .expect("pinned commit");
    assert_eq!(commit.len(), 40, "pinnedCommit must be a full commit id");
    assert!(
        commit
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_uppercase()),
        "pinnedCommit must be lowercase hex"
    );
    // Every rating cites evidence, and none of them may lean on adoption
    // counts: the discovery core rejects popularity-only reasoning, and a
    // rating recorded here is the one an operator reads instead of re-judging.
    for entry in entries(&catalog) {
        let id = entry["candidate"]["id"].as_str().expect("candidate id");
        let refs = entry["candidate"]["evidenceRefs"]
            .as_array()
            .expect("evidenceRefs is an array");
        assert!(!refs.is_empty(), "{id} cites no evidence");
    }
}

#[test]
fn the_registry_pins_the_catalog_so_an_edit_cannot_pass_unnoticed() {
    let registry = read_json("orchestration/integrations.json");
    let entry = registry["integrations"]
        .as_array()
        .expect("integrations is an array")
        .iter()
        .find(|entry| entry["id"] == "assistance.discovery")
        .expect("assistance.discovery must be registered before it is wired");
    assert_eq!(entry["effects"], json!([]));
    assert_eq!(
        entry["capabilityDeclaration"]["allowedEffects"],
        json!([]),
        "a proposal-only capability may declare no effect"
    );
    assert_eq!(
        entry["runtimeAdapter"], "assistance.discovery.compat",
        "the registry must name the adapter the dispatch table installs"
    );
    let inputs = entry["toolchainDigestEvidence"]["inputs"]
        .as_array()
        .expect("digest inputs");
    assert!(
        inputs.iter().any(|input| input == CATALOG_PATH),
        "the catalog must be pinned, or a rating could change without moving the digest"
    );
    assert_eq!(
        entry["capabilityDeclaration"]["implementation"]["toolchainDigests"]
            .as_array()
            .expect("toolchain digests")
            .len(),
        inputs.len(),
        "one digest per pinned input"
    );
}
