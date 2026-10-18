//! Dependency-keyed, in-process views over verified committed evidence (#264).
//!
//! A view is a disposable hint derived from Artifact Refs that A07/A08 and
//! `artifact_ref::verify_inputs` have already verified on this call. A cache
//! hit only skips re-deriving the view; it never skips verification and never
//! becomes authority. The key binds the view version, the snapshot identity and
//! every source ref in order, so any change to them yields a different key.
//! Current-checkout freshness is deliberately not part of the key.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde_json::Value;

use crate::capability::sha256_hex;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum ViewKind {
    /// `(artifactSchema, type)` per ref, in ref order.
    ArtifactCatalog,
    /// Lowercased UTF-8 (lossy) content per ref, in ref order.
    LexicalContent,
}

impl ViewKind {
    fn name(self) -> &'static str {
        match self {
            Self::ArtifactCatalog => "artifact-catalog",
            Self::LexicalContent => "lexical-content",
        }
    }

    fn version(self) -> u32 {
        match self {
            Self::ArtifactCatalog | Self::LexicalContent => 1,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct ViewKey(String);

impl ViewKey {
    pub(crate) fn new(kind: ViewKind, snapshot_identity: &str, refs: &[Value]) -> Self {
        Self::with_version(kind, kind.version(), snapshot_identity, refs)
    }

    fn with_version(kind: ViewKind, version: u32, snapshot_identity: &str, refs: &[Value]) -> Self {
        let mut material = format!("{}\0{version}\0{snapshot_identity}\0", kind.name());
        for artifact in refs {
            for field in [
                "artifactSchema",
                "type",
                "path",
                "sha256",
                "consumedSnapshotIdentity",
            ] {
                material.push_str(artifact[field].as_str().unwrap_or_default());
                material.push('\0');
            }
        }
        Self(sha256_hex(material.as_bytes()))
    }

    #[cfg(test)]
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug)]
pub(crate) struct ArtifactCatalogView {
    pub(crate) contracts: Vec<(String, String)>,
}

#[derive(Debug)]
pub(crate) struct LexicalContentView {
    pub(crate) lowered: Vec<String>,
}

#[derive(Debug)]
enum View {
    ArtifactCatalog(Arc<ArtifactCatalogView>),
    LexicalContent(Arc<LexicalContentView>),
}

/// Holds at most one view per kind: the one for the last key asked for. That
/// bounds memory for a long-lived MCP process while still letting repeated
/// queries against an unchanged committed run reuse the derived view.
#[derive(Debug, Default)]
pub(crate) struct QueryIndex {
    slots: Mutex<HashMap<ViewKind, (ViewKey, View)>>,
}

impl QueryIndex {
    pub(crate) fn artifact_catalog(
        &self,
        snapshot_identity: &str,
        refs: &[Value],
    ) -> Arc<ArtifactCatalogView> {
        let key = ViewKey::new(ViewKind::ArtifactCatalog, snapshot_identity, refs);
        if let Some(View::ArtifactCatalog(view)) = self.cached(ViewKind::ArtifactCatalog, &key) {
            return view;
        }
        let view = Arc::new(ArtifactCatalogView {
            contracts: refs
                .iter()
                .map(|artifact| {
                    (
                        artifact["artifactSchema"]
                            .as_str()
                            .unwrap_or_default()
                            .into(),
                        artifact["type"].as_str().unwrap_or_default().into(),
                    )
                })
                .collect(),
        });
        self.store(key, View::ArtifactCatalog(view.clone()));
        view
    }

    pub(crate) fn lexical_content(
        &self,
        snapshot_identity: &str,
        refs: &[Value],
        contents: &[&[u8]],
    ) -> Arc<LexicalContentView> {
        let key = ViewKey::new(ViewKind::LexicalContent, snapshot_identity, refs);
        if let Some(View::LexicalContent(view)) = self.cached(ViewKind::LexicalContent, &key) {
            return view;
        }
        let view = Arc::new(LexicalContentView {
            lowered: contents
                .iter()
                .map(|bytes| String::from_utf8_lossy(bytes).to_lowercase())
                .collect(),
        });
        self.store(key, View::LexicalContent(view.clone()));
        view
    }

    fn cached(&self, kind: ViewKind, key: &ViewKey) -> Option<View> {
        let slots = self.slots.lock().ok()?;
        let (stored, view) = slots.get(&kind)?;
        if stored != key {
            return None;
        }
        Some(match view {
            View::ArtifactCatalog(view) => View::ArtifactCatalog(view.clone()),
            View::LexicalContent(view) => View::LexicalContent(view.clone()),
        })
    }

    fn store(&self, key: ViewKey, view: View) {
        let kind = match view {
            View::ArtifactCatalog(_) => ViewKind::ArtifactCatalog,
            View::LexicalContent(_) => ViewKind::LexicalContent,
        };
        // A poisoned lock means a panic mid-update; drop every view and keep
        // serving by rebuilding. Views are hints, so losing them is harmless.
        let mut slots = self.slots.lock().unwrap_or_else(|poisoned| {
            let mut slots = poisoned.into_inner();
            slots.clear();
            self.slots.clear_poison();
            slots
        });
        slots.insert(kind, (key, view));
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    const SNAPSHOT: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn refs() -> Vec<Value> {
        vec![
            json!({"schema":"code-intel-artifact-ref.v1","artifactSchema":"s.v1","type":"repository.snapshot","path":"objects/a.json","sha256":"1".repeat(64),"consumedSnapshotIdentity":SNAPSHOT}),
            json!({"schema":"code-intel-artifact-ref.v1","artifactSchema":"t.v1","type":"inventory.rg","path":"objects/b.json","sha256":"2".repeat(64),"consumedSnapshotIdentity":SNAPSHOT}),
        ]
    }

    #[test]
    fn identical_inputs_produce_a_byte_stable_key() {
        let first = ViewKey::new(ViewKind::LexicalContent, SNAPSHOT, &refs());
        let second = ViewKey::new(ViewKind::LexicalContent, SNAPSHOT, &refs());
        assert_eq!(first, second);
        assert_eq!(first.as_str().len(), 64);
        assert_ne!(
            first,
            ViewKey::new(ViewKind::ArtifactCatalog, SNAPSHOT, &refs())
        );
    }

    #[test]
    fn every_bound_input_invalidates_the_key() {
        let base = ViewKey::new(ViewKind::LexicalContent, SNAPSHOT, &refs());
        let mut digest = refs();
        digest[1]["sha256"] = json!("3".repeat(64));
        let mut consumed = refs();
        consumed[0]["consumedSnapshotIdentity"] = json!("b".repeat(64));
        let mut reordered = refs();
        reordered.swap(0, 1);
        for (label, key) in [
            (
                "digest",
                ViewKey::new(ViewKind::LexicalContent, SNAPSHOT, &digest),
            ),
            (
                "consumed snapshot",
                ViewKey::new(ViewKind::LexicalContent, SNAPSHOT, &consumed),
            ),
            (
                "ref order",
                ViewKey::new(ViewKind::LexicalContent, SNAPSHOT, &reordered),
            ),
            (
                "snapshot identity",
                ViewKey::new(ViewKind::LexicalContent, &"c".repeat(64), &refs()),
            ),
            (
                "view version",
                ViewKey::with_version(ViewKind::LexicalContent, 2, SNAPSHOT, &refs()),
            ),
        ] {
            assert_ne!(base, key, "{label} must change the view key");
        }
    }

    #[test]
    fn an_unchanged_key_reuses_the_view_and_a_changed_one_rebuilds() {
        let index = QueryIndex::default();
        let contents: [&[u8]; 2] = [b"Alpha", b"BETA"];
        let first = index.lexical_content(SNAPSHOT, &refs(), &contents);
        assert_eq!(first.lowered, ["alpha", "beta"]);
        let again = index.lexical_content(SNAPSHOT, &refs(), &contents);
        assert!(
            Arc::ptr_eq(&first, &again),
            "identical key must reuse the view"
        );

        let mut changed = refs();
        changed[0]["sha256"] = json!("4".repeat(64));
        let rebuilt = index.lexical_content(SNAPSHOT, &changed, &[b"Gamma", b"BETA"]);
        assert!(!Arc::ptr_eq(&first, &rebuilt));
        assert_eq!(rebuilt.lowered, ["gamma", "beta"]);
    }

    #[test]
    fn a_poisoned_cache_degrades_to_a_rebuild() {
        let index = QueryIndex::default();
        let catalog = index.artifact_catalog(SNAPSHOT, &refs());
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = index.slots.lock().unwrap();
            panic!("poison the cache");
        }));
        let rebuilt = index.artifact_catalog(SNAPSHOT, &refs());
        assert!(!Arc::ptr_eq(&catalog, &rebuilt));
        assert_eq!(rebuilt.contracts, catalog.contracts);
        assert!(Arc::ptr_eq(
            &rebuilt,
            &index.artifact_catalog(SNAPSHOT, &refs())
        ));
    }
}
