# Final 69-item commitment reconciliation

This is the human-readable projection of `orchestration/evidence/final-commitment-reconciliation.json`.
The JSON is authoritative; this table must be regenerated or updated in the same change and is checked byte-for-byte after newline normalization.

Source plan: `docs/plans/adr-0010-execution-plan.md`<br>
Source SHA-256: `8ba922970a66f55087ec9711f16e71e4ca1af492ec79da0554d0718b0e580d33`<br>
Tickets: **69**

## Status totals

| Claim status | Count |
| --- | ---: |
| implemented_blocked | 1 |
| implemented_verified | 60 |
| retirement_blocked | 8 |

## Itemized reconciliation

| Ticket | Claim status | Independent verdict | Blockers | Evidence artifacts |
| --- | --- | --- | --- | --- |
| A00 — `compatibility.parity-baseline` | implemented_verified | verified | — | `crates/code-intel-cli/tests/capability_exec.rs` |
| A01 — `capability.runtime-exec` | implemented_verified | verified | — | `crates/code-intel-cli/tests/capability_exec.rs` |
| A02 — `repository.snapshot-identity` | implemented_verified | verified | — | `crates/code-intel-cli/tests/snapshot_identity.rs` |
| A03 — `artifact.ref-verify` | implemented_verified | verified | — | `crates/code-intel-cli/tests/artifact_ref.rs` |
| A04 — `evidence.admissibility-validate` | implemented_verified | verified | — | `crates/code-intel-cli/tests/evidence_admissibility.rs` |
| A05 — `authority.transition-gate` | implemented_verified | verified | — | `crates/code-intel-cli/tests/authority_transition.rs` |
| A09 — `run.dag-coordinate` | implemented_verified | verified | — | `crates/code-intel-cli/tests/dag_run.rs` |
| A06 — `artifact.stage-write` | implemented_verified | verified | — | `crates/code-intel-cli/tests/staged_artifact.rs` |
| A07 — `run.commit` | implemented_verified | verified | — | `crates/code-intel-cli/tests/run_commit.rs` |
| A08 — `artifact.index-committed-only` | implemented_verified | verified | — | `crates/code-intel-cli/tests/artifact_index.rs` |
| B01 — `provider.repowise-adapt` | implemented_verified | verified | — | `crates/code-intel-cli/tests/repowise_adapter.rs` |
| B02 — `provider.graph-adapt` | implemented_verified | verified | — | `crates/code-intel-cli/tests/graph_adapter.rs` |
| B03 — `provider.sentrux-adapt` | implemented_verified | verified | — | `crates/code-intel-cli/tests/sentrux_adapter.rs` |
| B04 — `provider.codenexus-adapt` | implemented_verified | verified | — | `crates/code-intel-cli/tests/codenexus_adapter.rs` |
| B05 — `repository.survival-scan` | implemented_verified | verified | — | `crates/code-intel-cli/tests/survival_scan.rs` |
| B06 — `advisory.workflow-recommend` | implemented_verified | verified | — | `crates/code-intel-cli/tests/method_select.rs` |
| B07 — `integration.registry-reconcile` | implemented_verified | verified | — | `crates/code-intel-cli/tests/capability_exec.rs`<br>`orchestration/integrations.json` |
| B08 — `evidence.native-code` | implemented_verified | verified | — | `crates/code-intel-cli/tests/native_code_evidence.rs` |
| B09 — `diagnosis.hospital` | implemented_verified | verified | — | `crates/code-intel-cli/tests/hospital_diagnosis.rs` |
| B10 — `doctor.envelope-adapt` | implemented_verified | verified | — | `crates/code-intel-cli/tests/doctor_envelope.rs` |
| C00 — `governance.ponytail-gate` | implemented_verified | verified | — | `crates/code-intel-cli/tests/ponytail_gate.rs` |
| C01 — `method.catalog` | implemented_verified | verified | — | `crates/code-intel-cli/tests/method_catalog.rs` |
| C02 — `method.select` | implemented_verified | verified | — | `crates/code-intel-cli/tests/method_select.rs` |
| C03 — `internalization.record-engine` | implemented_verified | verified | — | `crates/code-intel-cli/tests/internalization_record.rs` |
| C04 — `assistance.discover` | implemented_verified | verified | — | `crates/code-intel-cli/tests/assistance_discovery.rs` |
| C05 — `decision.gap-detect` | implemented_verified | verified | — | `crates/code-intel-cli/tests/decision_gap.rs` |
| C06 — `decision.request-response-port` | implemented_verified | verified | — | `crates/code-intel-cli/tests/decision_port.rs` |
| C07 — `decision.record` | implemented_verified | verified | — | `crates/code-intel-cli/tests/decision_record.rs` |
| R01 — `internalization.repowise-record` | implemented_verified | verified | — | `crates/code-intel-cli/tests/internalization_record.rs`<br>`docs/internalization-advisory-candidates.md` |
| R02 — `internalization.graph-record` | implemented_verified | verified | — | `crates/code-intel-cli/tests/internalization_record.rs`<br>`docs/internalization-advisory-candidates.md` |
| R03 — `internalization.sentrux-record` | implemented_verified | verified | — | `crates/code-intel-cli/tests/internalization_record.rs`<br>`docs/internalization-advisory-candidates.md` |
| R04 — `internalization.codenexus-record` | implemented_verified | verified | — | `crates/code-intel-cli/tests/internalization_record.rs`<br>`docs/internalization-advisory-candidates.md` |
| R05 — `internalization.repomix-record` | implemented_verified | verified | — | `crates/code-intel-cli/tests/internalization_record.rs`<br>`docs/internalization-advisory-candidates.md` |
| R06 — `internalization.native-code-evidence-record` | implemented_verified | verified | — | `crates/code-intel-cli/tests/internalization_record.rs`<br>`docs/internalization-advisory-candidates.md` |
| R07 — `internalization.cocoindex-record` | implemented_verified | verified | — | `crates/code-intel-cli/tests/internalization_record.rs`<br>`docs/internalization-advisory-candidates.md` |
| R08 — `internalization.github-research-record` | implemented_verified | verified | — | `crates/code-intel-cli/tests/internalization_record.rs`<br>`docs/internalization-advisory-candidates.md` |
| R09 — `internalization.rg-record` | implemented_verified | verified | — | `crates/code-intel-cli/tests/internalization_record.rs`<br>`docs/internalization-r09-r26-records.md` |
| R10 — `internalization.git-record` | implemented_verified | verified | — | `crates/code-intel-cli/tests/internalization_record.rs`<br>`docs/internalization-r09-r26-records.md` |
| R11 — `internalization.tree-sitter-v-record` | implemented_verified | verified | — | `crates/code-intel-cli/tests/internalization_record.rs`<br>`docs/internalization-r09-r26-records.md` |
| R12 — `internalization.greenfield-record` | implemented_verified | verified | — | `crates/code-intel-cli/tests/internalization_record.rs`<br>`docs/internalization-r09-r26-records.md` |
| R13 — `internalization.openspec-record` | implemented_verified | verified | — | `crates/code-intel-cli/tests/internalization_record.rs`<br>`docs/internalization-r09-r26-records.md` |
| R14 — `internalization.spec-kit-record` | implemented_verified | verified | — | `crates/code-intel-cli/tests/internalization_record.rs`<br>`docs/internalization-r09-r26-records.md` |
| R15 — `internalization.matt-flow-record` | implemented_verified | verified | — | `crates/code-intel-cli/tests/internalization_record.rs`<br>`docs/internalization-r09-r26-records.md` |
| R16 — `internalization.gstack-record` | implemented_verified | verified | — | `crates/code-intel-cli/tests/internalization_record.rs`<br>`docs/internalization-r09-r26-records.md` |
| R17 — `internalization.qiaomu-goal-record` | implemented_verified | verified | — | `crates/code-intel-cli/tests/internalization_record.rs`<br>`docs/internalization-r09-r26-records.md` |
| R18 — `internalization.agent-loops-record` | implemented_verified | verified | — | `crates/code-intel-cli/tests/internalization_record.rs`<br>`docs/internalization-r09-r26-records.md` |
| R19 — `internalization.metaharness-record` | implemented_verified | verified | — | `crates/code-intel-cli/tests/internalization_record.rs`<br>`docs/internalization-r09-r26-records.md` |
| R20 — `internalization.yao-meta-skill-record` | implemented_verified | verified | — | `crates/code-intel-cli/tests/internalization_record.rs`<br>`docs/internalization-r09-r26-records.md` |
| R21 — `internalization.ponytail-record` | implemented_verified | verified | — | `crates/code-intel-cli/tests/internalization_record.rs`<br>`docs/internalization-r09-r26-records.md` |
| R22 — `internalization.mattpocock-skills-record` | implemented_verified | verified | — | `crates/code-intel-cli/tests/internalization_record.rs`<br>`docs/internalization-r09-r26-records.md` |
| R23 — `internalization.linear-record` | implemented_verified | verified | — | `crates/code-intel-cli/tests/internalization_record.rs`<br>`docs/internalization-r09-r26-records.md` |
| R24 — `internalization.obsidian-record` | implemented_verified | verified | — | `crates/code-intel-cli/tests/internalization_record.rs`<br>`docs/internalization-r09-r26-records.md` |
| R25 — `internalization.llm-wiki-record` | implemented_verified | verified | — | `crates/code-intel-cli/tests/internalization_record.rs`<br>`docs/internalization-r09-r26-records.md` |
| R26 — `internalization.my-code-machine-record` | implemented_verified | verified | — | `crates/code-intel-cli/tests/internalization_record.rs`<br>`docs/internalization-r09-r26-records.md` |
| D01 — `project.orientation` | implemented_verified | verified | — | `crates/code-intel-cli/tests/project_orientation.rs` |
| D02 — `project.orientation-benchmark` | implemented_verified | verified | — | `crates/code-intel-cli/tests/project_orientation_benchmark.rs` |
| D03 — `understanding.quadrant` | implemented_verified | verified | — | `crates/code-intel-cli/tests/understanding_quadrant.rs` |
| D04 — `delivery.light-speed-measure` | implemented_verified | verified | — | `crates/code-intel-cli/tests/delivery_light_speed.rs` |
| E00 — `compatibility.retirement-gate` | implemented_verified | verified | — | `crates/code-intel-cli/tests/compatibility_retirement_gate.rs` |
| E01 — `compatibility.retirement-ticket-template` | implemented_verified | verified | — | `crates/code-intel-cli/tests/compatibility_retirement_ticket_template.rs` |
| E02 — `compatibility.retire-recommender-branch` | retirement_blocked | blocked | E00 decision remains blocked<br>deletionExecuted=false<br>retired=false | `orchestration/retirements/e02-recommender/status.json`<br>`docs/compatibility-retire-recommender-branch.md` |
| E03 — `compatibility.retire-provider-preflight-branch` | retirement_blocked | blocked | E00 decision remains blocked<br>deletionExecuted=false<br>retired=false | `orchestration/retirements/e03-provider-preflight/status.json`<br>`docs/compatibility-retire-provider-preflight-branch.md` |
| E04 — `compatibility.retire-codenexus-direct-branch` | retirement_blocked | blocked | E00 decision remains blocked<br>deletionExecuted=false<br>retired=false | `orchestration/retirements/e04-codenexus-direct/status.json`<br>`docs/compatibility-retire-codenexus-direct-branch.md` |
| E05 — `compatibility.retire-publication-branch` | retirement_blocked | blocked | E00 decision remains blocked<br>deletionExecuted=false<br>retired=false | `orchestration/retirements/e05-publication/status.json`<br>`docs/compatibility-retire-publication-branch.md` |
| E07 — `compatibility.retire-native-code-branch` | retirement_blocked | blocked | E00 decision remains blocked<br>deletionExecuted=false<br>retired=false | `orchestration/retirements/e07-native-code/status.json`<br>`docs/compatibility-retire-native-code-branch.md` |
| E08 — `compatibility.retire-hospital-branch` | retirement_blocked | blocked | E00 decision remains blocked<br>deletionExecuted=false<br>retired=false | `orchestration/retirements/e08-hospital/status.json`<br>`docs/compatibility-retire-hospital-branch.md` |
| E09 — `compatibility.retire-doctor-wrapper-branch` | retirement_blocked | blocked | E00 decision remains blocked<br>deletionExecuted=false<br>retired=false | `orchestration/retirements/e09-doctor-wrapper/status.json`<br>`docs/compatibility-retire-doctor-wrapper-branch.md` |
| E10 — `compatibility.retire-index-branch` | retirement_blocked | blocked | E00 decision remains blocked<br>deletionExecuted=false<br>retired=false | `orchestration/retirements/e10-index/status.json`<br>`docs/compatibility-retire-index-branch.md` |
| E06 — `compatibility.facade-finalize` | implemented_blocked | blocked | independent audit implementation approved, but final facade approval remains blocked<br>E02-E05 and E07-E10 retirement dependencies are not completed<br>current audit exits 2 with approvalEligible=false and independentApproval=null | `Invoke-CompatibilityFacadeFinalize.ps1`<br>`scripts/tests/test-compatibility-facade-finalize.ps1`<br>`orchestration/facade-finalize-policy.v1.json`<br>`orchestration/schemas/code-intel-compatibility-facade-finalize.v1.schema.json`<br>`docs/compatibility-facade-finalize.md` |
