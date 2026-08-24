//! Rust port of E02's PowerShell retirement-packet trio:
//! `New-RecommenderRetirementPacket.ps1`, `Test-RecommenderRetirementPacket.ps1`,
//! and `Restore-RecommenderLegacyBranch.ps1`. Pilot for #341's 22-file
//! `legacy/tools/compatibility` migration (AGENTS.md's language direction).
//! Split across four files to stay under the repo's `god_files` line-count
//! rule, along the PS1 originals' own script boundaries: this file
//! (generate/verify), `recommender_retirement_restore` (Restore-),
//! `recommender_retirement_diff` (the deletion-diff computation shared by
//! generate and its own tests), and `recommender_retirement_shared`
//! (constants and primitives common to all three).
//!
//! Preserves the PS1 originals' safety semantics exactly, not just their
//! shape: [`generate`] refuses (returns `Err`) to write anything but the
//! true current blocker set -- it never reports a fabricated `"approved"`
//! decision -- and [`verify`] locks in the expected blocker list as a
//! regression check, same as the scripts it replaces.
//!
//! Byte-for-byte parity with the PowerShell packet is not a goal (see
//! `frozen_manifest_projection`'s module doc): the two DO need to agree with
//! `compatibility_retirement_gate`'s own hash of the necessity trace, since
//! that hash is independently recomputed and checked by the gate -- so
//! `necessity_trace_sha256` below deliberately mirrors the gate's exact
//! `json!` call so both sides serialize through the same serde_json
//! (alphabetical-key) canonicalization and agree without hand-tuned field
//! ordering.

use std::fs;
use std::path::Path;

use serde_json::{json, Value};

use crate::capability::{self, sha256_hex};
use crate::capability_inventory;
use crate::frozen_manifest_projection::frozen_source_identity;
use crate::recommender_retirement_diff::{build_result_text, compute_delete_hunks};
use crate::recommender_retirement_restore::{restore_legacy_branch, RestoreMode};
use crate::recommender_retirement_shared::{
    artifact_ref, expected_blockers, frozen_set, write_json_file, BRANCH_ID,
    CURRENT_FUNCTIONS_START, CURRENT_INVOCATION_START, DEFAULT_SOURCE_REVISION, FUNCTIONS_END,
    INVOCATION_END, LEGACY_CAPABILITY, REPLACEMENT_ID, RETIREMENT_ID,
};

struct Evidence<'a> {
    out_dir: &'a Path,
    snapshot_identity: &'a str,
}

impl Evidence<'_> {
    fn add(&self, name: &str, class: &str, details: Value) -> Result<Value, String> {
        let value = json!({
            "schema": "code-intel-compatibility-retirement-evidence.v1",
            "snapshotIdentity": self.snapshot_identity,
            "id": format!("e02.{name}"),
            "evidenceClass": class,
            "retirementId": RETIREMENT_ID,
            "legacyBranchId": BRANCH_ID,
            "replacementCapabilityId": REPLACEMENT_ID,
            "details": details,
        });
        let relative = format!("evidence/{name}.json");
        write_json_file(&self.out_dir.join(&relative), &value)?;
        artifact_ref(
            self.out_dir,
            "code-intel-compatibility-retirement-evidence.v1",
            "compatibility.retirement-evidence",
            &relative,
            self.snapshot_identity,
        )
    }
}

/// Run a subprocess purely as a pass/fail verification gate, exactly as the
/// PS1 generator does with `pwsh`/`cargo test` -- these are downstream test
/// commands, not PowerShell orchestration logic being ported.
fn command_passes(program: &str, args: &[&str], cwd: &Path) -> bool {
    std::process::Command::new(program)
        .args(args)
        .current_dir(cwd)
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn placeholder_snapshot(identity: &str) -> Value {
    json!({
        "identity": identity,
        "repoIdentity": format!("content-v1:{}", "c".repeat(64)),
        "head": "unversioned",
        "workingTreePolicy": "explicit_overlay",
        "scope": ["."],
        "inputDigest": "d".repeat(64),
    })
}

/// Generate a fresh E02 packet at `out_dir` (which must not already exist),
/// mirroring `New-RecommenderRetirementPacket.ps1`. Never writes to
/// `run-code-intel.ps1`; the deletion diff is recorded evidence, not an
/// applied change. Returns `Err` (never a fabricated "approved" packet) if
/// verification fails, the legacy inline recommender is still present, the
/// unrelated provider-preflight branch changed, or the gate somehow
/// reports anything other than the true current blocked state.
pub(crate) fn generate(
    out_dir: &Path,
    evaluated_at: u64,
    repo_root: &Path,
    pipeline_repo_root: &Path,
    manifest: Option<&Path>,
) -> Result<Value, String> {
    if out_dir.exists() {
        return Err(format!(
            "packet output must be exclusive: {}",
            out_dir.display()
        ));
    }
    fs::create_dir_all(out_dir.join("evidence")).map_err(|e| e.to_string())?;

    let brief_ok = command_passes(
        "pwsh",
        &[
            "-NoLogo",
            "-NoProfile",
            "-File",
            repo_root
                .join("scripts/tests/test-workflow-recommendation-brief.ps1")
                .to_str()
                .unwrap(),
        ],
        repo_root,
    );
    let facade_parity_ok = command_passes(
        "cargo",
        &[
            "test",
            "-p",
            "code-intel",
            "--test",
            "capability_exec",
            "advisory_workflow_recommend_runs_through_a01_with_zero_effects_and_facade_parity",
            "--quiet",
        ],
        pipeline_repo_root,
    );
    let authority_isolation_ok = command_passes(
        "cargo",
        &[
            "test",
            "-p",
            "code-intel",
            "--test",
            "authority_transition",
            "recommender_direct_commit_requires_explicit_approved_authority_event",
            "--quiet",
        ],
        pipeline_repo_root,
    );
    if !(brief_ok && facade_parity_ok && authority_isolation_ok) {
        return Err("recommender parity or authority verification failed".into());
    }
    let verification = json!({
        "brief": brief_ok,
        "facadeParity": facade_parity_ok,
        "authorityIsolation": authority_isolation_ok,
    });

    let run_text = fs::read_to_string(repo_root.join("run-code-intel.ps1"))
        .map_err(|e| format!("read run-code-intel.ps1: {e}"))?;
    let legacy_inline_absent = !run_text.contains("function Invoke-WorkflowStackDetector")
        && !run_text.contains("function Get-CodeMetrics");
    let provider_preflight_present = run_text.contains("Invoke-RepowiseProviderProbe.ps1");
    if !legacy_inline_absent {
        return Err("legacy inline recommender is still present".into());
    }
    if !provider_preflight_present {
        return Err("unrelated provider-preflight branch changed; E02 refuses to proceed".into());
    }

    let snapshot_identity = frozen_source_identity(&frozen_set(), repo_root, pipeline_repo_root)?;
    let expiry = evaluated_at + 30 * 86_400;
    let evidence = Evidence {
        out_dir,
        snapshot_identity: &snapshot_identity,
    };

    let replacement = evidence.add(
        "replacement-atom",
        "replacement_atom",
        json!({"outcome":"passed","status":"production_ready","capability":REPLACEMENT_ID,"verification":verification}),
    )?;
    let golden = evidence.add(
        "golden-parity",
        "golden_parity",
        json!({"outcome":"passed","assertionCount":4,"command":"scripts/tests/test-workflow-recommendation-brief.ps1"}),
    )?;
    let contract = evidence.add(
        "contract-parity",
        "contract_parity",
        json!({"outcome":"passed","assertionCount":8,"command":"cargo test -p code-intel --test capability_exec advisory_workflow_recommend_runs_through_a01_with_zero_effects_and_facade_parity"}),
    )?;
    let effects = evidence.add(
        "effect-parity",
        "effect_parity",
        json!({"outcome":"passed","assertionCount":3,"declaredEffects":[],"observedEffects":[],"noAutoInit":true}),
    )?;
    let registry = evidence.add(
        "registry-reconciliation",
        "registry_reconciliation",
        json!({"outcome":"passed","registryParticipantId":LEGACY_CAPABILITY,"replacementCapabilityId":REPLACEMENT_ID,"status":"deleted","providerPreflightUntouched":provider_preflight_present}),
    )?;
    let window = evidence.add(
        "compatibility-window",
        "compatibility_window",
        json!({"outcome":"blocked","startedAt":evaluated_at,"observedThrough":evaluated_at,"minimumDays":30,"checkedAt":evaluated_at,"expiresAt":expiry,"blocker":"no completed 30-day compatibility observation window"}),
    )?;

    let rehearsal_relative = format!("work/e02-recommender-rollback-{evaluated_at}");
    let rehearsal_root = repo_root.join(&rehearsal_relative);
    let rollback_command = format!(
        "pwsh -NoLogo -NoProfile -File tools/compatibility/Restore-RecommenderLegacyBranch.ps1 -RehearsalRoot {rehearsal_relative}"
    );
    restore_legacy_branch(
        repo_root,
        RestoreMode::Rehearsal(rehearsal_root),
        DEFAULT_SOURCE_REVISION,
    )
    .map_err(|e| format!("rollback rehearsal failed: {e}"))?;
    let rollback = evidence.add(
        "rollback-execution",
        "rollback_execution",
        json!({"outcome":"passed","command":rollback_command.clone(),"executedAt":evaluated_at,"exitCode":0,"target":format!("{rehearsal_relative}/run-code-intel.ps1"),"replacementChanged":false}),
    )?;
    let usage = evidence.add(
        "usage-observation",
        "usage_observation",
        json!({"outcome":"blocked","startedAt":evaluated_at,"endedAt":evaluated_at,"totalInvocations":0,"legacyInvocations":0,"replacementInvocations":0,"blocker":"no production usage observation exists"}),
    )?;
    // Mirrors compatibility_retirement_gate::check_necessity_and_dependencies's
    // own json! call byte-for-byte so both sides' serde_json canonicalization
    // (alphabetical keys) agree without hand-tuned field ordering.
    let necessity_trace_sha = sha256_hex(
        &serde_json::to_vec(&json!({
            "retirementId": RETIREMENT_ID,
            "legacyBranchId": BRANCH_ID,
            "replacementCapabilityId": REPLACEMENT_ID,
        }))
        .unwrap(),
    );
    let necessity = evidence.add(
        "c00-necessity",
        "c00_necessity",
        json!({"outcome":"passed","decision":"admit","changeId":RETIREMENT_ID,"necessityTraceSha256":necessity_trace_sha}),
    )?;
    let snapshot_dependency = evidence.add(
        "dependency-repo-snapshot",
        "dependency_approval",
        json!({"outcome":"passed","dependencyId":"repo.snapshot","status":"approved","reviewer":"e02-author"}),
    )?;
    let d02_dependency = evidence.add(
        "dependency-d02-clean-machine",
        "dependency_approval",
        json!({"outcome":"blocked","dependencyId":"project.orientation-benchmark","status":"pending","reviewer":"independent-verifier-required","blocker":"D02 clean-machine repetition is not complete"}),
    )?;

    let subject = json!({
        "legacyBranch": {"capabilityId":LEGACY_CAPABILITY,"branchId":BRANCH_ID,"callPath":format!("run-code-intel.ps1::{BRANCH_ID}"),"affectedFiles":["run-code-intel.ps1"],"owner":"executor-recommender","registryParticipantId":LEGACY_CAPABILITY},
        "replacement": {"capabilityId":REPLACEMENT_ID,"implementationId":"advisory.workflow-recommend.compat","dependencies":["repo.snapshot","project.orientation-benchmark"],"atomEvidence":replacement},
        "parity": {"golden":golden.clone(),"contract":contract.clone(),"effects":effects.clone()},
        "registryReconciliation": registry,
        "compatibilityWindow": window,
        "rollback": {"command":rollback_command.clone(),"executionEvidence":rollback.clone()},
        "usageObservation": usage.clone(),
        "necessityEvidence": necessity,
        "dependencyStates": [snapshot_dependency, d02_dependency],
        "lineReductionEvidence": false,
    });
    let independent = evidence.add(
        "independent-approval",
        "independent_approval",
        json!({"outcome":"blocked","approved":false,"authorIndependent":false,"subjectSha256":"0".repeat(64),"reviewer":"independent-verifier-required","authorityEvent":{},"blocker":"no independent repository-governed approval exists"}),
    )?;
    let manifest_value = json!({
        "schema": "code-intel-compatibility-retirement-manifest.v1",
        "snapshotIdentity": snapshot_identity,
        "retirementId": RETIREMENT_ID,
        "approvalSubject": subject,
        "independentApproval": independent,
    });
    write_json_file(
        &out_dir.join("compatibility-retirement-manifest.json"),
        &manifest_value,
    )?;
    let manifest_ref = artifact_ref(
        out_dir,
        "code-intel-compatibility-retirement-manifest.v1",
        "compatibility.retirement-manifest",
        "compatibility-retirement-manifest.json",
        &snapshot_identity,
    )?;

    let gate_declaration = capability::declaration_for("compatibility.retirement-gate", manifest)?;
    let inputs = vec![
        manifest_ref.clone(),
        replacement,
        golden.clone(),
        contract.clone(),
        effects.clone(),
        registry,
        window,
        rollback.clone(),
        usage.clone(),
        necessity,
        snapshot_dependency,
        d02_dependency,
        independent,
    ];
    let gate_request = json!({
        "schema": "code-intel-capability-request.v1",
        "capability": "compatibility.retirement-gate",
        "contractVersion": 1,
        "implementation": gate_declaration["implementation"],
        "snapshot": placeholder_snapshot(&snapshot_identity),
        "options": {"evaluatedAt": evaluated_at},
        "inputs": inputs,
        "effectPolicy": {"allowedEffects": gate_declaration["allowedEffects"]},
    });
    write_json_file(&out_dir.join("e00-request.json"), &gate_request)?;
    let gate_out = out_dir.join("gate-out");
    let gate_outcome = capability::exec_in_process(
        "compatibility.retirement-gate",
        &gate_request,
        &gate_out,
        Some(out_dir),
        manifest,
        capability_inventory::execute,
    );
    if gate_outcome.exit_code != 0 {
        return Err(format!(
            "E00 execution failed: {}",
            gate_outcome.diagnostic.unwrap_or_default()
        ));
    }
    let decision: Value = serde_json::from_slice(
        &fs::read(gate_out.join("compatibility-retirement-decision.json"))
            .map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    if decision["decision"] != "blocked" {
        return Err(
            "E02 must not proceed without real usage, D02 clean-machine evidence, and independent approval"
                .into(),
        );
    }

    let base_text = run_text.replace("\r\n", "\n").replace('\r', "\n");
    let hunks = compute_delete_hunks(
        &base_text,
        &[
            (CURRENT_FUNCTIONS_START, FUNCTIONS_END),
            (CURRENT_INVOCATION_START, INVOCATION_END),
        ],
    )?;
    let result_text = build_result_text(&base_text, &hunks);
    let hunk_values: Vec<Value> = hunks
        .iter()
        .map(|hunk| {
            json!({
                "addedLines": [],
                "deletedLines": hunk.deleted_lines,
                "newLines": 0,
                "newStart": hunk.new_start,
                "oldLines": hunk.old_lines,
                "oldStart": hunk.old_start,
            })
        })
        .collect();
    let patch_files = json!([{
        "baseBlobSha256": sha256_hex(base_text.as_bytes()),
        "baseText": base_text,
        "hunks": hunk_values,
        "path": "run-code-intel.ps1",
        "resultBlobSha256": sha256_hex(result_text.as_bytes()),
        "resultText": result_text,
    }]);
    let patch_sha = sha256_hex(&serde_json::to_vec(&patch_files).unwrap());
    let deletion_diff = json!({
        "schema": "code-intel-compatibility-retirement-deletion-diff.v1",
        "snapshotIdentity": snapshot_identity,
        "retirementId": RETIREMENT_ID,
        "legacyBranchId": BRANCH_ID,
        "affectedFiles": ["run-code-intel.ps1"],
        "deletionsOnly": true,
        "summary": "Proposed deletion is limited to the retired inline recommender adapter markers; provider-preflight and all other branches are excluded. Summary is non-authoritative.",
        "patch": {"algorithm":"replayable-delete-only-v1","sha256":patch_sha,"files":patch_files},
    });
    write_json_file(
        &out_dir.join("compatibility-retirement-deletion-diff.json"),
        &deletion_diff,
    )?;
    let diff_ref = artifact_ref(
        out_dir,
        "code-intel-compatibility-retirement-deletion-diff.v1",
        "compatibility.retirement-deletion-diff",
        "compatibility-retirement-deletion-diff.json",
        &snapshot_identity,
    )?;
    let decision_ref = artifact_ref(
        out_dir,
        "code-intel-compatibility-retirement-decision.v1",
        "compatibility.retirement-decision",
        "gate-out/compatibility-retirement-decision.json",
        &snapshot_identity,
    )?;

    let ticket = json!({
        "schema": "code-intel-compatibility-retirement-ticket-template.v1",
        "snapshotIdentity": snapshot_identity,
        "ticketId": "ticket-e02-retire-recommender-branch",
        "retirementId": RETIREMENT_ID,
        "legacyBranch": {"capabilityId":LEGACY_CAPABILITY,"branchId":BRANCH_ID,"callPath":format!("run-code-intel.ps1::{BRANCH_ID}")},
        "replacement": {"capabilityId":REPLACEMENT_ID,"dependencies":["repo.snapshot","project.orientation-benchmark"]},
        "affectedFiles": ["run-code-intel.ps1"],
        "evidence": {"golden":golden,"contract":contract,"effects":effects,"usage":usage,"rollbackRehearsal":rollback,"deletionDiff":diff_ref},
        "source": {"retirementDecision":decision_ref,"retirementManifest":manifest_ref},
        "owner": "executor-recommender",
        "verifier": "independent-verifier-required",
        "observationExpiry": expiry,
        "status": "draft",
        "authorityBoundary": "template_only_no_approval_or_deletion_authority",
    });
    write_json_file(
        &out_dir.join("compatibility-retirement-ticket.json"),
        &ticket,
    )?;
    let ticket_declaration =
        capability::declaration_for("compatibility.retirement-ticket-template", manifest)?;
    let ticket_ref = artifact_ref(
        out_dir,
        "code-intel-compatibility-retirement-ticket-template.v1",
        "compatibility.retirement-ticket-template",
        "compatibility-retirement-ticket.json",
        &snapshot_identity,
    )?;
    let e01_request = json!({
        "schema": "code-intel-capability-request.v1",
        "capability": "compatibility.retirement-ticket-template",
        "contractVersion": 1,
        "implementation": ticket_declaration["implementation"],
        "snapshot": placeholder_snapshot(&snapshot_identity),
        "options": {"evaluatedAt": evaluated_at},
        "inputs": [ticket_ref, manifest_ref, decision_ref, diff_ref],
        "effectPolicy": {"allowedEffects": ticket_declaration["allowedEffects"]},
    });
    write_json_file(&out_dir.join("e01-request.json"), &e01_request)?;
    let e01_out = out_dir.join("e01-out");
    let e01_outcome = capability::exec_in_process(
        "compatibility.retirement-ticket-template",
        &e01_request,
        &e01_out,
        Some(out_dir),
        manifest,
        capability_inventory::execute,
    );
    let e01_text = e01_outcome.diagnostic.clone().unwrap_or_default();
    fs::write(out_dir.join("e01-stderr.txt"), &e01_text).map_err(|e| e.to_string())?;
    if e01_outcome.exit_code != 65 || !e01_text.contains("ticket requires an approved E00 decision")
    {
        return Err(format!(
            "E01 must validate the replayable patch and reject only because the real E00 decision is blocked: exit={} output={e01_text}",
            e01_outcome.exit_code
        ));
    }

    let status = json!({
        "schema": "code-intel-compatibility-retirement-execution-status.v1",
        "retirementId": RETIREMENT_ID,
        "decision": "blocked",
        "deletionExecuted": false,
        "retired": false,
        "blockers": decision["blockers"],
        "gainLedgerProjection": decision["gainLedgerProjection"],
        "boundary": "E02 generated a complete draft packet but has no approval or deletion authority while any E00 blocker remains.",
    });
    write_json_file(&out_dir.join("status.json"), &status)?;
    Ok(status)
}

fn read_json(packet_root: &Path, relative: &str) -> Result<Value, String> {
    let path = packet_root.join(relative);
    if !path.is_file() {
        return Err(format!("packet file is missing: {relative}"));
    }
    serde_json::from_slice(&fs::read(&path).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
}

/// Verify a packet at `packet_root` against the live tree, mirroring
/// `Test-RecommenderRetirementPacket.ps1`. Locks in the current expected
/// blocker set as a regression check -- a packet that silently gained
/// "approved" without a matching gate-code and evidence change fails this,
/// same as the script it replaces.
pub(crate) fn verify(
    packet_root: &Path,
    repo_root: &Path,
    pipeline_repo_root: &Path,
) -> Result<Value, String> {
    let ticket = read_json(packet_root, "compatibility-retirement-ticket.json")?;
    let manifest = read_json(packet_root, "compatibility-retirement-manifest.json")?;
    let decision = read_json(
        packet_root,
        "gate-out/compatibility-retirement-decision.json",
    )?;
    let diff = read_json(packet_root, "compatibility-retirement-deletion-diff.json")?;

    let snapshot_identity = frozen_source_identity(&frozen_set(), repo_root, pipeline_repo_root)?;
    for artifact in [&ticket, &manifest, &decision, &diff] {
        if artifact["snapshotIdentity"] != Value::String(snapshot_identity.clone()) {
            return Err("E02 packet is stale relative to its frozen source set".into());
        }
    }
    let status = read_json(packet_root, "status.json")?;
    let expected_call_path = format!("run-code-intel.ps1::{BRANCH_ID}");

    if ticket["legacyBranch"]["capabilityId"] != LEGACY_CAPABILITY
        || ticket["legacyBranch"]["branchId"] != BRANCH_ID
        || ticket["legacyBranch"]["callPath"] != expected_call_path
    {
        return Err(
            "E02 ticket must identify exactly the inline recommender branch and call path".into(),
        );
    }
    let affected = ticket["affectedFiles"]
        .as_array()
        .map(Vec::len)
        .unwrap_or(0);
    if affected != 1 || ticket["affectedFiles"][0] != "run-code-intel.ps1" {
        return Err("E02 ticket cannot include provider-preflight or any file other than run-code-intel.ps1".into());
    }
    let diff_affected = diff["affectedFiles"].as_array().map(Vec::len).unwrap_or(0);
    if diff_affected != 1
        || diff["affectedFiles"][0] != "run-code-intel.ps1"
        || diff["legacyBranchId"] != BRANCH_ID
        || diff["deletionsOnly"] != true
    {
        return Err("E02 deletion diff exceeds the single recommender branch boundary".into());
    }
    if ticket["replacement"]["capabilityId"] != REPLACEMENT_ID
        || manifest["approvalSubject"]["replacement"]["capabilityId"] != REPLACEMENT_ID
    {
        return Err("E02 replacement must remain advisory.workflow-recommend".into());
    }
    if manifest["approvalSubject"]["legacyBranch"]["branchId"] != BRANCH_ID {
        return Err("E00 manifest branch differs from the E02 ticket".into());
    }
    let manifest_affected = manifest["approvalSubject"]["legacyBranch"]["affectedFiles"]
        .as_array()
        .map(Vec::len)
        .unwrap_or(0);
    if manifest["approvalSubject"]["legacyBranch"]["callPath"] != expected_call_path
        || manifest_affected != 1
        || manifest["approvalSubject"]["legacyBranch"]["affectedFiles"][0] != "run-code-intel.ps1"
    {
        return Err(
            "E00 approval subject does not bind the exact E02 call path and file set".into(),
        );
    }
    let patch_files = diff["patch"]["files"].as_array().map(Vec::len).unwrap_or(0);
    let hunk_count = diff["patch"]["files"][0]["hunks"]
        .as_array()
        .map(Vec::len)
        .unwrap_or(0);
    if diff["patch"]["algorithm"] != "replayable-delete-only-v1"
        || patch_files != 1
        || diff["patch"]["files"][0]["path"] != "run-code-intel.ps1"
        || hunk_count != 2
    {
        return Err("E02 deletion proof is not the bounded replayable two-hunk patch".into());
    }
    for hunk in diff["patch"]["files"][0]["hunks"]
        .as_array()
        .unwrap_or(&vec![])
    {
        let added = hunk["addedLines"].as_array().map(Vec::len).unwrap_or(0);
        let old_lines = hunk["oldLines"].as_i64().unwrap_or(0);
        if hunk["newLines"] != 0 || added != 0 || old_lines <= 0 {
            return Err(
                "E02 deletion proof contains an addition/replacement or an empty deletion".into(),
            );
        }
    }
    let e01_rejection =
        fs::read_to_string(packet_root.join("e01-stderr.txt")).map_err(|e| e.to_string())?;
    if !e01_rejection.contains("ticket requires an approved E00 decision") {
        return Err(
            "E01 did not validate the patch before rejecting the blocked E00 decision".into(),
        );
    }

    let evidence_dir = packet_root.join("evidence");
    let mut evidence = Vec::new();
    for entry in fs::read_dir(&evidence_dir).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        if entry.path().extension().and_then(|e| e.to_str()) == Some("json") {
            evidence.push(
                serde_json::from_slice::<Value>(
                    &fs::read(entry.path()).map_err(|e| e.to_string())?,
                )
                .map_err(|e| e.to_string())?,
            );
        }
    }
    if evidence.len() != 12 {
        return Err("E02 must contain exactly twelve closed E00 evidence artifacts".into());
    }
    for item in &evidence {
        if item["legacyBranchId"] != BRANCH_ID || item["replacementCapabilityId"] != REPLACEMENT_ID
        {
            return Err("E02 evidence crossed a branch or replacement boundary".into());
        }
    }

    let actual_blockers: Vec<String> = decision["blockers"]
        .as_array()
        .map(|values| {
            values
                .iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let mut sorted_actual = actual_blockers.clone();
    sorted_actual.sort();
    let mut sorted_expected: Vec<String> = expected_blockers()
        .into_iter()
        .map(str::to_string)
        .collect();
    sorted_expected.sort();
    if decision["decision"] != "blocked" || sorted_actual != sorted_expected {
        return Err("E02 decision must retain the current compatibility, usage, D02, and independent-approval blockers".into());
    }
    if status["decision"] != "blocked"
        || status["deletionExecuted"] != false
        || status["retired"] != false
    {
        return Err("blocked E02 packet cannot claim deletion or retirement".into());
    }

    let run_text =
        fs::read_to_string(repo_root.join("run-code-intel.ps1")).map_err(|e| e.to_string())?;
    if !run_text.contains("Invoke-RepowiseProviderProbe.ps1") {
        return Err("provider-preflight marker is absent; E02 scope was violated".into());
    }

    Ok(json!({
        "ok": true,
        "retirementId": status["retirementId"],
        "decision": status["decision"],
        "deletionExecuted": status["deletionExecuted"],
        "retired": status["retired"],
        "evidenceCount": evidence.len(),
        "affectedFiles": ticket["affectedFiles"],
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn scratch_dir(name: &str) -> std::path::PathBuf {
        let nonce = COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "recommender-packet-test-{name}-{}-{nonce}",
            std::process::id()
        ))
    }

    #[test]
    fn verify_reports_a_missing_packet_file() {
        let dir = scratch_dir("missing-packet-file");
        fs::create_dir_all(&dir).unwrap();
        let result = verify(&dir, Path::new("."), Path::new(".."));
        assert!(result.unwrap_err().contains("packet file is missing"));
        fs::remove_dir_all(&dir).ok();
    }

    /// Full generate -> verify round trip against the real repository.
    /// Ignored by default: spawns `pwsh` and two `cargo test` subprocesses,
    /// so it is slow and must be run explicitly, not on every `cargo test`.
    /// Run with: cargo test -p code-intel --bin code-intel
    /// recommender_retirement_packet::tests::generate_then_verify_round_trips_against_the_real_repository
    /// -- --ignored --nocapture
    #[test]
    #[ignore]
    fn generate_then_verify_round_trips_against_the_real_repository() {
        let pipeline_repo_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let repo_root = pipeline_repo_root.join("legacy");
        let out_dir = scratch_dir("real-e02-packet");
        let evaluated_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let status = generate(
            &out_dir,
            evaluated_at,
            &repo_root,
            &pipeline_repo_root,
            None,
        )
        .expect("generate should produce the true current blocked packet");
        assert_eq!(status["decision"], "blocked");
        assert_eq!(status["deletionExecuted"], false);
        assert_eq!(status["retired"], false);

        let report = verify(&out_dir, &repo_root, &pipeline_repo_root)
            .expect("verify should accept the packet generate just produced");
        assert_eq!(report["ok"], true);
        assert_eq!(report["decision"], "blocked");
        assert_eq!(report["evidenceCount"], 12);

        fs::remove_dir_all(&out_dir).ok();
    }
}
