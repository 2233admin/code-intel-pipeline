use std::path::PathBuf;

use crate::committed_evidence_controller::{CommittedAuthority, CommittedReceipt};
use crate::workspace_advisory_controller::{AdvisoryBasis, WorkspaceAuthority, WorkspaceSource};

#[test]
fn committed_and_workspace_controllers_have_distinct_inhabited_authority_types() {
    // Workspace authority is constructible for each advisory basis without a
    // published run. Committed authority exists only as a receipt bound to
    // completed-index identity — the two types are intentionally disjoint so
    // a result cannot silently cross the trust boundary.
    let workspace = WorkspaceAuthority::Advisory(AdvisoryBasis::WorkingTree {
        repo_root: PathBuf::from("."),
    });
    assert_eq!(workspace.basis().source(), WorkspaceSource::WorkingTree);

    let git_history = WorkspaceAuthority::Advisory(AdvisoryBasis::GitHistory {
        repo_root: PathBuf::from("."),
        revspec: "HEAD~1..HEAD".into(),
    });
    assert_eq!(git_history.basis().source(), WorkspaceSource::GitHistory);

    let stale = WorkspaceAuthority::Advisory(AdvisoryBasis::StaleCommittedSnapshot {
        repo: "example".into(),
        run: "run-1".into(),
        recorded_snapshot_identity: "snap-old".into(),
        current_snapshot_identity: Some("snap-new".into()),
    });
    assert_eq!(
        stale.basis().source(),
        WorkspaceSource::StaleCommittedSnapshot
    );

    // CommittedAuthority is inhabited only via Receipt. Construct a minimal
    // receipt so the type is inhabited in tests without going through the
    // completed-only index (that path is covered by integration tests).
    let committed = CommittedAuthority::Receipt(CommittedReceipt::for_test(
        "example", "run-1", "run-id", "snap",
    ));
    assert_eq!(committed.receipt().repo(), "example");
    assert_eq!(committed.receipt().run(), "run-1");

    // Type-level guard: if someone unifies the two authority enums, this
    // dual-parameter helper stops compiling.
    fn keep_distinct(_committed: CommittedAuthority, _workspace: WorkspaceAuthority) {}
    let _ = keep_distinct as fn(CommittedAuthority, WorkspaceAuthority);
}
