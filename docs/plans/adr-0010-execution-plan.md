# ADR 0010 Execution Plan

Status: proposed, not implemented
Scope: ADR 0009 runtime convergence plus ADR 0010 tool-neutral engineering-intelligence core
Rule: every ticket below delivers exactly one independently testable capability. A checked-in definition, schema, ADR, or plan is not implementation evidence.

## Outcome and completion evidence

This plan is complete only when the compatibility facade can produce a committed artifact run through versioned capability envelopes, provider evidence is validated before it becomes fact, recommendations cannot cross authority boundaries, CodeNexus is replaceable through a Pipeline-owned port, the promised method/reuse/decision/orientation capabilities have executable contracts and proving tests, and PowerShell retirement is supported by parity evidence. `run-code-intel.ps1` remains the public compatibility facade until the final retirement gate passes.

Fresh evidence inspected while writing this plan:

- ADR 0009 originally states its accepted contract did not itself change runtime execution or publication; the current dirty worktree now contains partial, uncommitted implementation attempts that must be verified against this plan rather than treated as absent or complete.
- ADR 0010 says convergence is future work and forbids a big-bang rewrite.
- `run-code-intel.ps1` still contains the workflow recommender and directly invokes provider preflight and `Invoke-CodeNexusLite.ps1`.
- `run-code-intel.ps1` currently creates a `.staging-<nonce>` directory, promotes it, rewrites staged path text, and writes `run-complete.json` last; `update-code-intel-index.ps1` rejects staging directories and missing/invalid completion markers. These are partial, dirty-worktree implementations, not yet proof of A06-A08 atomicity, interruption safety, portable identity, envelope coherence, or independent verification.
- `scripts/tests/test-transactional-publication.ps1` currently exercises staging exclusion, marker shape, path rewriting, and index admission. It is useful draft regression evidence, but it is untracked and has not independently proven the complete publication contract or all interruption phases.
- `crates/code-intel-cli/src/providers.rs` and `orchestration/integrations.json` currently contain dirty-worktree provider/manifest reconciliation, including canonical `codenexus/lite`, manifest lookup/drift checks, registered `diagnosis.hospital`, and doctor/runtime entries. These are partial/unverified implementations: several routes remain compatibility commands and they do not yet constitute the A04 shared admissibility engine or B01-B04 conformance.
- `docs/architecture/reference-capability-map.md` currently inventories 12 manifest integrations plus drift/reference entries and explicitly says it is not adoption approval or health proof. It is an untracked audit draft and becomes input to B07/R01-R26, not completion evidence for those tickets.
- Current dirty/untracked tests and docs, including `scripts/tests/test-integration-orchestration.ps1`, `scripts/tests/test-skill-development-benchmark.ps1`, ADR 0010, and the files above, predate or run concurrently with this plan; this plan neither claims them verified nor rewrites them.

## Delivery rules

1. Execute tickets in dependency order; parallelize only tickets whose dependencies are complete.
2. Add a failing proving test before changing behavior. Preserve golden artifacts before each extraction.
3. One ticket may change several files but must expose only one new capability responsibility.
4. Existing PowerShell stays as an adapter until the replacement has contract, parity, and rollback evidence.
5. No recommendation, model output, or provider success becomes an Engineering Fact, Adoption Decision, or Committed Engineering Plan by implication.
6. Every external adoption must have a Reuse Record before production enablement.
7. Every ticket requires an implementer and an independent verifier; the verifier must not author the implementation under review.

## Dependency spine

```text
A00 parity baseline -> A01 real inventory.rg envelope executor
  -> A02 snapshot identity -> A03 Artifact Ref verifier
      -> A04 core evidence admissibility -> A05 authority-transition gate
      -> A06 staged artifact writer
      -> A09 run DAG coordinator -> A07 Run Commit -> A08 committed-run index guard

A04 -> B01 Repowise adapter
    -> B02 graph-provider adapter
    -> B03 Sentrux adapter
    -> B04 CodeNexus adapter -> B05 survival-scanner fallback
A05 -> B06 workflow recommender Advisory Atom
A04 -> B08 Native Code Evidence atom
A05 + B01..B03 + B05 + B08 -> B09 diagnosis.hospital atom
A09 + B01..B04 + B08 -> B07 production registry reconciliation -> B10 doctor envelope adapter/extraction

A05 -> C00 executable Ponytail gate
A04 -> C01 Method Card catalog -> C02 deterministic method selection
A04 + A05 -> C03 Internalization Record engine -> R01..R26 per-executable/provider/reference migrations or approved retirement/out-of-scope records
C03 + C01 + A05 -> C04 assistance discovery
A05 -> C05 Decision Gap -> C06 Decision request/response port -> C07 Decision Record
A02 + A03 + B05 -> D01 Project Orientation -> D02 corpus orientation benchmark
D01 -> D03 understanding quadrant
A07 + D02 -> D04 Light-Speed measurement

A01 + A07 + B04 + B06 + B07 + D02 -> E00 retirement gate engine
E00 -> E01 retirement-ticket template
  -> E02/E03/E04/E05/E07/E08/E09 single-branch retirements
  -> E10 index retirement (after E05 publication retirement)
  -> E06 final facade/DAG gate
```

The first implementation slice is A00-A05. It establishes regression evidence, a real `inventory.rg` runtime envelope, portable snapshot and Artifact Ref checks, provider admissibility, and authority enforcement before any provider or recommender extraction.

## Atomic tickets

### A00 — `compatibility.parity-baseline`

- **Owner / boundary:** test-engineer; owns immutable golden inputs and normalized current-output fixtures, not production orchestration.
- **Dependencies:** none.
- **Affected files (initial):** `tests/fixtures/parity/**` (new), `scripts/tests/test-code-intel-pipeline.ps1`, `scripts/tests/test-integration-orchestration.ps1`.
- **Acceptance criteria:** representative clean, dirty, provider-unavailable, domain-fail, and partial-evidence runs have path/time-normalized golden machine artifacts; fixture update requires an explicit review reason; no production behavior changes.
- **Smallest proving test:** run one fixture twice and assert byte-identical normalized output plus a deliberate mismatch rejection.
- **Compatibility / rollback:** additive test-only capability; rollback is deleting the fixture harness without touching runtime.
- **Ponytail Necessity Trace:** required by the no-big-bang strangler rule to prove later adapters preserve current behavior.
- **Economic implementation lane:** PowerShell test harness using existing JSON utilities; no dependency and no new runtime.
- **Independent verifier condition:** verifier reproduces one golden run in a fresh temp directory and confirms normalization does not hide verdict, provenance, or missing evidence.

### A01 — `capability.runtime-exec`

- **Owner / boundary:** executor; owns `code-intel capability exec` request/result I/O and exit-code mapping, not capability-specific logic.
- **Dependencies:** A00.
- **Affected files (initial):** `crates/code-intel-cli/src/main.rs`, new `crates/code-intel-cli/src/capability.rs`, `orchestration/integrations.json`, new contract test fixture(s).
- **Acceptance criteria:** stdin or request-file accepts exactly one v1 request; declaration/request coherence is checked; stdout contains exactly one v1 result; diagnostics use stderr; legal `status × verdict × exitCode` combinations are enforced; the real `inventory.rg` compatibility implementation executes through the path and preserves its normalized A00 artifact.
- **Smallest proving test:** execute `inventory.rg` through a v1 request against the A00 fixture and assert a schema-valid envelope, exit 0, and normalized artifact parity; then change the capability id and assert exit 64 with no partial result artifact.
- **Compatibility / rollback:** existing scripts remain callable; feature flag/explicit subcommand selects the new executor; rollback routes facade to the old command.
- **Ponytail Necessity Trace:** runtime envelopes are the minimum mechanism needed to turn ADR 0009 from vocabulary into enforcement.
- **Economic implementation lane:** small Rust control-plane module reusing current CLI parsing and JSON dependencies; no workflow engine.
- **Independent verifier condition:** verifier checks stdout purity, all documented exit classes, and confirms no capability logic leaked into the executor.

### A02 — `repository.snapshot-identity`

- **Owner / boundary:** executor; owns portable identity of consumed repository inputs, not artifact publication.
- **Dependencies:** A01.
- **Affected files (initial):** new Rust snapshot module, `crates/code-intel-cli/src/main.rs`, snapshot fixtures, facade adapter.
- **Acceptance criteria:** identity binds repository identity, HEAD, working-tree policy, scope, and input digest; absolute paths and timestamps do not affect identity; dirty overlays are explicit.
- **Smallest proving test:** copy the same repository snapshot to two paths and assert equal identity; change one scoped byte and assert inequality.
- **Compatibility / rollback:** old timestamp run folders remain navigation views; rollback omits new identity consumption but never rewrites old artifacts.
- **Ponytail Necessity Trace:** closes stale/current evidence confusion with one shared identity primitive.
- **Economic implementation lane:** Rust hashing over Git and existing inventory; no new store.
- **Independent verifier condition:** cross-path, dirty-tree, sub-scope, and missing-Git cases pass documented rules.

### A03 — `artifact.ref-verify`

- **Owner / boundary:** executor; owns Artifact Ref schema, digest, and consumed-snapshot validation, not artifact production.
- **Dependencies:** A02.
- **Affected files (initial):** Rust artifact-ref module, envelope schema/tests, compatibility artifact reader.
- **Acceptance criteria:** a referenced payload is accepted only when schema, SHA-256, type, location, and snapshot identity agree; machine-local location is never identity.
- **Smallest proving test:** mutate referenced bytes after ref creation and assert exit 65.
- **Compatibility / rollback:** legacy path-only input is adapter-only and marked unverified; rollback retains legacy reader.
- **Ponytail Necessity Trace:** one verifier prevents every downstream atom from reimplementing trust checks.
- **Economic implementation lane:** shared Rust library function and table-driven fixtures.
- **Independent verifier condition:** verifier covers digest, schema, snapshot, missing file, and relocation cases.

### A04 — `evidence.admissibility-validate`

- **Owner / boundary:** executor; owns tool-neutral validation of Observed Evidence against the Pipeline Evidence Provider Port, not provider-native probes, adapter translation, or business authority.
- **Dependencies:** A02, A03.
- **Affected files (initial):** new provider-port/admissibility schema under `orchestration/schemas/`, shared Rust validator, conformance fixture protocol, negative tests.
- **Acceptance criteria:** validation requires provider and implementation identity, source revision or endpoint identity, consumed snapshot, freshness, completeness, payload schema, provenance, and failure semantics; malformed, stale-for-policy, snapshot-mismatched, digest-mismatched, or incomplete-as-complete output is rejected; the core contains no Repowise, graph, Sentrux, or CodeNexus branching.
- **Smallest proving test:** validate a provider-neutral good fixture, then independently mutate snapshot identity and payload digest and assert exit 65/domain unknown without producing an Engineering Fact.
- **Compatibility / rollback:** initially callable beside legacy probes; rollback disables enforcement while preserving emitted observations and conformance evidence.
- **Ponytail Necessity Trace:** one provider-neutral admissibility engine avoids four copies of the same trust boundary.
- **Economic implementation lane:** shared Rust validation functions over A02/A03 primitives and existing JSON Schema dependencies.
- **Independent verifier condition:** verifier uses a synthetic provider unknown to the registry and confirms identical validation semantics plus fail-closed mismatch handling.

### A05 — `authority.transition-gate`

- **Owner / boundary:** architect/executor; owns explicit transitions among Observed Evidence, Engineering Fact, Derived Engineering Model, proposal, Adoption Decision, and Committed Engineering Plan; it does not choose product priorities.
- **Dependencies:** A04.
- **Affected files (initial):** new authority schema/policy under `orchestration/`, new Rust policy module, result-envelope provenance extension if required, authority tests.
- **Acceptance criteria:** allowed transitions and required approver/evidence fields are machine-enforced; LLM/provider/recommender output may create only observations or proposals; Adoption Decision and Committed Engineering Plan require an explicit recorded authority event; rejected transitions fail closed and preserve unrelated analysis.
- **Smallest proving test:** attempt to promote a recommender proposal directly to a committed plan and assert rejection; repeat with an explicit approved authority event and assert acceptance.
- **Compatibility / rollback:** initially audit-only beside current outputs, then enforce behind a flag after parity; rollback returns to audit-only while preserving emitted authority records.
- **Ponytail Necessity Trace:** this is the smallest guard that makes the promised “LLM/tool is not a fact or commitment source” boundary real.
- **Economic implementation lane:** deterministic policy table in the Rust core; no OPA or policy server.
- **Independent verifier condition:** verifier tests every edge in the transition table, including replay, missing approver, unknown evidence, and unrelated-branch continuation.

### A09 — `run.dag-coordinate`

- **Owner / boundary:** executor; owns dependency resolution, ready-node scheduling, result propagation, resume state, and terminal run outcome for the declared capability DAG; it does not implement atoms, validate provider payload semantics, or publish final runs.
- **Dependencies:** A01, A02, A03.
- **Affected files (initial):** new Rust coordinator/DAG module, capability declarations in `orchestration/integrations.json`, run-state schema, orchestration parity tests, facade adapter.
- **Acceptance criteria:** rejects missing and cyclic dependencies; runs independent ready nodes concurrently; passes only verified Artifact Refs; preserves domain fail versus process failure; skips only dependency-blocked descendants; resumes completed nodes by deterministic identity; emits a complete run manifest for A07.
- **Smallest proving test:** run the real `repo.snapshot -> inventory.rg` two-node DAG through envelopes, assert ordered dependency execution and A00 parity, then add an independent node and prove it still completes when a sibling branch domain-fails.
- **Compatibility / rollback:** `run-code-intel.ps1` remains the outer facade and may select legacy sequential orchestration; rollback switches the facade route without deleting coordinator state.
- **Ponytail Necessity Trace:** a single coordinator is necessary to make the declared graph executable; no workflow engine, queue, or database is introduced.
- **Economic implementation lane:** in-process Rust topological scheduler over the existing registry and filesystem run state.
- **Independent verifier condition:** verifier tests cycle/missing-dependency rejection, concurrency, branch-local failure, resume, deterministic order, and manifest completeness.

### A06 — `artifact.stage-write`

- **Owner / boundary:** executor; owns validated writes into a unique staging directory, not final publication.
- **Dependencies:** A03.
- **Affected files (initial):** new Rust artifact writer, capability runtime integration, staging tests.
- **Acceptance criteria:** writes are content-addressed, validated before return, and leave no final run visible; failure reports observed effects and cleans only owned staging.
- **Smallest proving test:** inject a schema failure and assert no final directory or completion marker exists.
- **Compatibility / rollback:** facade continues legacy writes until A07; rollback removes staged path selection.
- **Ponytail Necessity Trace:** transactional publication needs one shared writer, not per-capability file logic.
- **Economic implementation lane:** filesystem primitives only; no database/CAS service.
- **Independent verifier condition:** verifier tests interrupted, duplicate-content, and out-of-scope path writes.

### A07 — `run.commit`

- **Owner / boundary:** executor; owns atomic promotion and writing `run-complete.json` last, not artifact generation or indexing.
- **Dependencies:** A06, A09.
- **Affected files (initial):** Rust publication module, facade publication adapter, run-commit schema/tests.
- **Acceptance criteria:** all refs validate before promotion; completion marker is last; failed promotion cannot appear complete; marker binds run manifest digest and snapshot.
- **Smallest proving test:** kill/inject failure before marker write and assert run is uncommitted and recoverable.
- **Compatibility / rollback:** legacy timestamp runs remain readable but are marked legacy-uncommitted; rollback uses the prior writer behind facade.
- **Ponytail Necessity Trace:** closes the known partial-run/index corruption risk with a single transaction boundary.
- **Economic implementation lane:** same-volume rename plus fsync/replace semantics; no transaction service.
- **Independent verifier condition:** verifier exercises interruption at each publication phase and checks marker ordering.

### A08 — `artifact.index-committed-only`

- **Owner / boundary:** executor; owns index admission, not run production.
- **Dependencies:** A07.
- **Affected files (initial):** index reader/writer in Rust, `update-code-intel-index.ps1` adapter, tests.
- **Acceptance criteria:** only valid completion markers enter the index; incomplete/forged markers are ignored with diagnosis; index is rebuildable.
- **Smallest proving test:** place complete and staged runs side by side and assert only the complete run is indexed.
- **Compatibility / rollback:** legacy indexing available under explicit compatibility mode.
- **Ponytail Necessity Trace:** makes Run Commit meaningful to consumers.
- **Economic implementation lane:** reuse current index format where possible; no migration database.
- **Independent verifier condition:** rebuild result equals incremental result and rejects forged manifest digests.

### B01 — `provider.repowise-adapt`

- **Owner / boundary:** executor; owns Repowise-native health/index/docs translation into the Evidence Provider Port, not admissibility policy or Repowise internals.
- **Dependencies:** A04.
- **Affected files (initial):** `crates/code-intel-cli/src/providers.rs`, Repowise adapter, `scripts/tests/test-code-intel-provider.ps1`, conformance fixtures, facade route.
- **Acceptance criteria:** health remains distinct from evidence; index and docs declare different completeness/freshness/effects; quota failure cannot disable index status; all output passes A04 before fact promotion; production no longer runs a test file as its validator.
- **Smallest proving test:** translate success, quota, and index-only fixtures; assert A04 accepts the good fixture and preserves quota as provider-unavailable/partial rather than local failure.
- **Compatibility / rollback:** current CLI/Python probe remains behind the adapter; rollback routes facade to legacy preflight/index-only behavior.
- **Ponytail Necessity Trace:** one thin adapter contains Repowise quirks without contaminating the provider-neutral core.
- **Economic implementation lane:** reuse existing provider registry and probe, adding only translation/conformance code.
- **Independent verifier condition:** verifier tests quota, missing CLI, stale index, successful-but-incomplete docs, and no-secret output.

### B02 — `provider.graph-adapt`

- **Owner / boundary:** executor; owns internal/external graph-provider translation into one architecture-graph port, not graph algorithms.
- **Dependencies:** A04.
- **Affected files (initial):** `crates/code-intel-cli/src/providers.rs`, graph adapter/fixtures, `orchestration/integrations.json`, graph tests.
- **Acceptance criteria:** internal Rust and external Understand-compatible outputs share one schema and provenance; current-snapshot binding is mandatory; fallback identity is explicit; stale graph presence never becomes current anatomy.
- **Smallest proving test:** present a valid graph from the wrong HEAD and assert A04 rejection, then accept a current-snapshot fixture.
- **Compatibility / rollback:** external graph remains explicit fallback; rollback selects legacy command without changing the port contract.
- **Ponytail Necessity Trace:** eliminates stale-graph ambiguity at one adapter seam rather than rewriting providers.
- **Economic implementation lane:** translation plus conformance fixtures over existing graph commands.
- **Independent verifier condition:** verifier swaps internal/external providers and tests stale, missing, partial, and current graphs.

### B03 — `provider.sentrux-adapt`

- **Owner / boundary:** executor; owns Sentrux collection/normalization translation into structural evidence, not structural policy or diagnosis.
- **Dependencies:** A04.
- **Affected files (initial):** `Invoke-SentruxAgentTool.ps1`, Sentrux adapter/schema/fixtures, normalization tests, integration registry.
- **Acceptance criteria:** every authoritative rule kind has normalized status/verdict/failure semantics; effects are declared; partial normalization remains incomplete/unknown; provider output passes A04 before diagnosis.
- **Smallest proving test:** an unrecognized authoritative rule kind yields incomplete/unknown rather than pass, while a complete fixture validates.
- **Compatibility / rollback:** bundled shim and current script remain provider implementations; rollback selects them through the same adapter.
- **Ponytail Necessity Trace:** contains existing normalization debt without duplicating Sentrux logic in the core.
- **Economic implementation lane:** strangler wrapper and tables before any script split.
- **Independent verifier condition:** verifier covers every rule kind, provider crash, partial output, and observed-effect mismatch.

### B04 — `provider.codenexus-adapt`

- **Owner / boundary:** executor; Pipeline owns the port/adapter; CodeNexus owns indexing, storage, retrieval, and impact semantics.
- **Dependencies:** A04.
- **Affected files (initial):** `orchestration/integrations.json`, CodeNexus port fixtures/adapter, `Invoke-CodeNexusLite.ps1`, `crates/code-intel-cli/src/providers.rs`.
- **Acceptance criteria:** full CodeNexus and lite compatibility output map to the same provenance-bearing port and pass A04; Pipeline neither imports CodeNexus internals nor shares its database; absence yields provider-unavailable, not fabricated empty facts.
- **Smallest proving test:** swap full and lite fixtures behind the same request and validate equal port shape with distinct provenance and snapshot identity.
- **Compatibility / rollback:** lite script remains fallback adapter; rollback selects it explicitly.
- **Ponytail Necessity Trace:** satisfies perception reuse without duplicating an intelligence platform.
- **Economic implementation lane:** thin adapter and conformance fixtures; do not revive the demoted worker unless evidence requires it.
- **Independent verifier condition:** verifier confirms process/storage decoupling, snapshot binding, provider swap, and absence semantics.

### B05 — `repository.survival-scan`

- **Owner / boundary:** executor; owns minimum repository identity/basic inventory/provider diagnosis only, never CodeNexus-equivalent graph intelligence.
- **Dependencies:** B04.
- **Affected files (initial):** existing inventory implementation or one small Rust atom, integration declaration, tests.
- **Acceptance criteria:** useful orientation bootstrap survives CodeNexus absence; output explicitly declares reduced completeness and cannot masquerade as structural perception.
- **Smallest proving test:** run with CodeNexus unavailable and assert basic evidence plus unknown structural verdict.
- **Compatibility / rollback:** current rg inventory remains adapter.
- **Ponytail Necessity Trace:** minimum survival ability is an ADR-owned boundary; richer duplication is forbidden.
- **Economic implementation lane:** reuse `rg`/Git, no new parser.
- **Independent verifier condition:** verifier proves no graph/impact claim appears in fallback output.

### B06 — `advisory.workflow-recommend`

- **Owner / boundary:** executor; owns tool-neutral recommendations with evidence/confidence/alternatives, never adoption or execution.
- **Dependencies:** A05, A02, A03.
- **Affected files (initial):** `OpenSpec-Detector.ps1`, new atom implementation/contract, `run-code-intel.ps1`, `scripts/tests/test-workflow-recommendation-brief.ps1`.
- **Acceptance criteria:** duplicated recommender logic is removed from the main runner; OpenSpec/spec-kit/gstack are candidates, not dependencies; output is a schema-valid Advisory Atom proposal; authority gate blocks automatic adoption/init.
- **Smallest proving test:** same fixture through standalone atom and facade yields parity; direct promotion to Adoption Decision is rejected.
- **Compatibility / rollback:** historical `-SkipOpenSpec/-AutoOpenSpec` flags map to adapter options; rollback invokes standalone script.
- **Ponytail Necessity Trace:** removes known duplication and runner coupling while preserving behavior.
- **Economic implementation lane:** extract existing PowerShell once, then wrap with A01; no algorithm rewrite.
- **Independent verifier condition:** verifier checks parity, no prompt/init side effect, alternatives, provenance, and authority rejection.

### B07 — `integration.registry-reconcile`

- **Owner / boundary:** architect/executor; owns reconciliation between declared capability registry and actual production invocation, not provider behavior.
- **Dependencies:** A09, B01, B02, B03, B04, B08.
- **Affected files (initial):** `orchestration/integrations.json`, `run-code-intel.ps1`, `Invoke-CodeIntelOrchestrator.ps1`, registry audit test.
- **Acceptance criteria:** every production participant is either declared with capability id, envelope, dependencies, effects, artifacts, and owner or removed from production invocation; audit explicitly covers doctor, diagnosis.hospital, Repomix, Native Code Evidence, cocoindex, GitHub Solution Research, Repowise, graph, Sentrux, CodeNexus, publication, and index; orphan declarations and undeclared invocations fail CI.
- **Smallest proving test:** insert an undeclared Repomix invocation fixture and assert audit failure, then register it and assert its dependency/effect metadata is required.
- **Compatibility / rollback:** audit-only report precedes enforcement; deleting an unused participant is preferred to speculative registration; rollback returns enforcement to warning without restoring deleted dead code.
- **Ponytail Necessity Trace:** prevents the runtime graph from diverging from its declared graph and forces register-or-delete decisions.
- **Economic implementation lane:** static registry/invocation audit using existing files and `rg`; no service discovery.
- **Independent verifier condition:** verifier traces each named production participant from facade call site to registry declaration or reviewed deletion evidence.

### B08 — `evidence.native-code`

- **Owner / boundary:** executor; owns extraction of the current built-in files/symbols/chunks/imports/scorecard/Agent Code Slice producer into one deterministic capability atom, not specialized semantic graph claims or external cocoindex behavior.
- **Dependencies:** A01, A02, A03, A04, A09.
- **Affected files (initial):** native code-evidence functions currently in `run-code-intel.ps1`, new atom module/adapter, Code Evidence schemas, A/B and contract tests, registry declaration.
- **Acceptance criteria:** atom consumes a Snapshot Identity, emits verified Artifact Refs for stable v1 artifacts, declares heuristic/parser coverage and effects, preserves current normalized artifacts, and returns unknown for unsupported precision rather than fabricating call-graph facts; facade invocation passes through A01/A09.
- **Smallest proving test:** run the representative native Code Evidence fixture through an envelope, assert A00 artifact parity and A03 refs, then feed an unsupported language construct and assert explicit coverage/unknown rather than false relationship evidence.
- **Compatibility / rollback:** embedded functions remain a compatibility implementation until parity and E07 approval; cocoindex remains a separate optional adapter/candidate.
- **Ponytail Necessity Trace:** this is an active drift-unregistered production producer and a prerequisite for a complete declared run DAG.
- **Economic implementation lane:** extract existing deterministic logic first; do not add a parser framework or rewrite algorithms.
- **Independent verifier condition:** verifier reruns code-evidence layer/A-B fixtures, checks snapshot/digest/effect boundaries, and audits unsupported-language claims.

### B09 — `diagnosis.hospital`

- **Owner / boundary:** executor; owns pure deterministic diagnosis, disposition, next-protocol, treatment, and surgery-plan routing from admitted evidence, not evidence collection, Markdown authority, or execution of treatment.
- **Dependencies:** A05, B01, B02, B03, B05, B08.
- **Affected files (initial):** `New-Hospital*`/`Get-Hospital*` functions currently in `run-code-intel.ps1`, new diagnosis atom, hospital JSON schema/fixtures, rendering separation tests, registry declaration.
- **Acceptance criteria:** consumes only A04-admitted Artifact Refs; authoritative missing/untrusted modalities yield unknown/fail-closed; the same facts produce the same machine diagnosis; Markdown is a rebuildable view; enrichment cannot override an untrusted authoritative diagnosis; output executes through A01/A09.
- **Smallest proving test:** execute a provider-quota plus missing-current-graph fixture through the envelope and assert deterministic fail-closed diagnosis/next protocol, then rebuild Markdown without changing the machine verdict.
- **Compatibility / rollback:** embedded Hospital functions remain adapter implementation until parity and E08 approval; current stable JSON fields remain readable.
- **Ponytail Necessity Trace:** Hospital is an active-required registered capability and must become a real atom for the final DAG to be truthful.
- **Economic implementation lane:** extract pure decision/rendering seams and reuse existing fixtures; no rules engine.
- **Independent verifier condition:** verifier covers every diagnosis precedence case, missing/untrusted evidence, deterministic replay, rendering independence, and facade parity.

### B10 — `doctor.envelope-adapt`

- **Owner / boundary:** executor; owns extraction/adaptation of repository/tool/provider/manifest readiness probes into one envelope-emitting doctor capability, not provider admissibility or scan execution.
- **Dependencies:** A01, A02, A03, B07.
- **Affected files (initial):** `check-code-intel-tools.ps1`, stable wrapper doctor path, Rust `doctor` command/adapter, doctor schema/fixtures, registry/manifest tests.
- **Acceptance criteria:** doctor executes through A01, emits one schema-valid result plus Artifact Refs bound to the examined snapshot/environment policy, separates presence/readiness from conformance/admissibility, redacts secrets, validates the reconciled manifest, and preserves a shell-compatible bootstrap path.
- **Smallest proving test:** run doctor through an envelope on a fixture with present-but-nonconforming provider and manifest drift; assert readiness observations without fact promotion, redacted output, and nonzero domain diagnosis while stdout remains one result document.
- **Compatibility / rollback:** `check-code-intel-tools.ps1` remains the bootstrap adapter until E09 approval; rollback restores wrapper routing without changing the doctor contract.
- **Ponytail Necessity Trace:** doctor is active-required and currently outside the runtime envelope, leaving the public run DAG incomplete.
- **Economic implementation lane:** wrap/extract existing probes and Rust artifact-doctor code; no new health service.
- **Independent verifier condition:** verifier tests missing/present/nonconforming tools, manifest drift, provider health separation, secret redaction, stdout purity, and bootstrap rollback.

### C00 — `governance.ponytail-gate`

- **Owner / boundary:** architect/executor; owns executable Necessity Trace validation and Ponytail Value Filter admission, not prioritization or code-style judgment.
- **Dependencies:** A05.
- **Affected files (initial):** necessity-trace schema, deterministic gate module, capability declaration/PR fixture tests, CI integration.
- **Acceptance criteria:** every new artifact, dependency, abstraction, file, test, doc, or process change names an allowed current value source and the first sufficient solution rung; unsupported output is rejected; safety/evidence/verification requirements cannot be filtered out; bypass requires an explicit authority record with expiry.
- **Smallest proving test:** reject a new dependency justified only by “may be useful later,” accept an existing-utility reuse tied to a committed deliverable, and reject an attempt to delete a required verification gate.
- **Compatibility / rollback:** begin report-only on changed capabilities, then enforce for new production participants; rollback returns to report-only while retaining trace artifacts.
- **Ponytail Necessity Trace:** this ticket implements the value filter itself, closing the gap between glossary language and executable governance.
- **Economic implementation lane:** JSON Schema plus small rule table integrated with existing contract/CI checks; no policy platform.
- **Independent verifier condition:** verifier supplies addition, deletion, reuse, dependency, safety, speculative, and expired-bypass cases and confirms fail-closed behavior.

### C01 — `method.catalog`

- **Owner / boundary:** architect; owns Method Card schema/catalog and engineering meaning, not provider implementations.
- **Dependencies:** A04.
- **Affected files (initial):** new `orchestration/methods/` cards/schema, catalog loader/tests, docs link.
- **Acceptance criteria:** each card declares problem signals, required evidence, assumptions, deterministic steps, outputs, confidence rules, cost, contraindications, and implementation ports; seed cards cover root-cause analysis, FMEA, fault tree, critical path/PERT, value-stream/queue delay, PDCA, SPC, contract testing, and strangler migration without claiming they ran.
- **Smallest proving test:** validate all cards and reject one missing contraindications/confidence rule.
- **Compatibility / rollback:** catalog is additive data; no method is auto-selected.
- **Ponytail Necessity Trace:** one canonical catalog prevents prompt folklore and tool-owned methodology.
- **Economic implementation lane:** versioned JSON/YAML data plus schema; no methodology framework.
- **Independent verifier condition:** engineering reviewer confirms cards preserve established method preconditions and do not collapse distinct methods into slogans.

### C02 — `method.select`

- **Owner / boundary:** executor; owns deterministic matching of facts/gaps to Method Cards, not execution or commitment.
- **Dependencies:** C01, A05.
- **Affected files (initial):** method selector module, fixtures/tests.
- **Acceptance criteria:** selection is reproducible, explains matched signals/missing evidence/cost, can return none/unknown, and emits proposal only.
- **Smallest proving test:** a dependency-delay fixture selects critical-path/value-stream cards while insufficient evidence returns unknown.
- **Compatibility / rollback:** advisory-only opt-in; remove route to roll back.
- **Ponytail Necessity Trace:** operationalizes traditional engineering methods without an LLM choosing by taste.
- **Economic implementation lane:** rule table over cards and facts.
- **Independent verifier condition:** verifier tests false-positive, insufficient-evidence, tie, and deterministic ordering cases.

### C03 — `internalization.record-engine`

- **Owner / boundary:** dependency-expert; owns the Internalization Standard schema, lifecycle validation, and Reuse Record projections, not any individual adoption decision or package installation.
- **Dependencies:** A04, A05, C00.
- **Affected files (initial):** internalization/reuse schemas, record validator/store, NOTICE/provenance projection, lifecycle tests.
- **Acceptance criteria:** every record requires source revision, license obligations, adoption rung, owned boundary, compatibility and conformance evidence, measured benefit/cost, maintenance/security evidence, update policy/check date, owned modifications, rollback, exit, and retirement evidence/status; missing or expired required evidence blocks production enablement but not research; record state transitions require authority.
- **Smallest proving test:** reject enablement when conformance, measurement, update, or retirement/exit evidence is missing; accept a complete fixture and project a valid Reuse Record.
- **Compatibility / rollback:** audit-only inventory first; enforcement begins only after R01-R26 migrations or authority-approved retirement/out-of-scope records; rollback returns to audit-only without deleting records.
- **Ponytail Necessity Trace:** one executable Internalization Standard replaces bespoke reference prose and unmanaged copies.
- **Economic implementation lane:** repository data records and validator using existing schema tooling; no dependency management platform.
- **Independent verifier condition:** verifier tests every lifecycle evidence class, expiry, update, replacement, rollback, retirement, and authority transition independently.

### C04 — `assistance.discover`

- **Owner / boundary:** dependency-expert; owns comparable candidate dossiers for a proven capability gap, not adoption.
- **Dependencies:** C03, C01, A05.
- **Affected files (initial):** dossier schema, discovery atom, candidate fixtures/tests.
- **Acceptance criteria:** search begins only from a named Engineering Capability Gap; internal atoms, established methods, external tools, and docs are comparable; output includes fit/license/security/integration/reversibility and remains a proposal.
- **Smallest proving test:** gap fixture yields dossiers and cannot install or create an Adoption Decision.
- **Compatibility / rollback:** opt-in advisory atom; no external writes.
- **Ponytail Necessity Trace:** prevents speculative dependency search while enabling evidence-driven reuse.
- **Economic implementation lane:** deterministic local inventory first; optional network provider later under declared effects.
- **Independent verifier condition:** verifier confirms no candidate ranking relies on popularity alone and no mutation occurs.

### C05 — `decision.gap-detect`

- **Owner / boundary:** analyst; owns identification and branch-local blocking of choices facts cannot resolve, not questioning or recording answers.
- **Dependencies:** A05.
- **Affected files (initial):** Decision Gap schema, detector/policy tests.
- **Acceptance criteria:** gap names blocked decision, discoverable facts already checked, options/consequences, recommended answer, and affected branches; unrelated deterministic analysis continues.
- **Smallest proving test:** unresolved risk acceptance blocks only publication while inventory completes.
- **Compatibility / rollback:** initially emits supplemental artifact; no interactive dependency.
- **Ponytail Necessity Trace:** asks humans only for irreducible authority choices.
- **Economic implementation lane:** deterministic rules over missing authority/preconditions.
- **Independent verifier condition:** verifier distinguishes missing facts from genuine choices and tests branch locality.

### C06 — `decision.request-response-port`

- **Owner / boundary:** executor; owns a replaceable asynchronous request/response contract for one Decision Gap, not UI rendering, transport choice, answer authority, or persistence.
- **Dependencies:** C05.
- **Affected files (initial):** decision request/response schemas, port trait/interface, CLI/native UI adapter fixtures, correlation/timeout tests.
- **Acceptance criteria:** request carries gap id, one question, recommendation, evidence refs, options/consequences, authority needed, and expiry; response carries correlation id, chosen option/free-form answer, actor/authority provenance, and timestamp; CLI, native structured UI, or future service adapters are replaceable; timeout blocks only the dependent branch.
- **Smallest proving test:** send one gap through an in-memory adapter, correlate a valid response, reject stale/wrong-gap response, and prove an unrelated DAG branch completes while the request is pending.
- **Compatibility / rollback:** outside-tmux/native environments may use a plain-text adapter; rollback swaps adapters without changing gap or record schemas.
- **Ponytail Necessity Trace:** isolates the irreducible human-choice boundary without coupling Pipeline semantics to chat, tmux, or one host.
- **Economic implementation lane:** port plus in-memory/file adapter first; no message broker.
- **Independent verifier condition:** verifier substitutes a second adapter and covers timeout, replay, wrong actor, stale evidence, cancellation, and branch locality.

### C07 — `decision.record`

- **Owner / boundary:** executor; owns durable resolution and replay of one Decision Gap, not the interview UI or request transport.
- **Dependencies:** C06, A05, A07.
- **Affected files (initial):** Decision Record schema/store, authority linkage, tests.
- **Acceptance criteria:** record binds gap/request/response/evidence/snapshot, accepted choice, authority, consequences, freshness/reopen rule; replay prevents repeated questioning unless evidence changes.
- **Smallest proving test:** resolve once through C06, replay without question, then change bound evidence and require reopen.
- **Compatibility / rollback:** records are additive; invalid records are ignored with diagnosis.
- **Ponytail Necessity Trace:** eliminates repeated coordination delay and undocumented approval.
- **Economic implementation lane:** committed JSON artifact; UI remains replaceable.
- **Independent verifier condition:** verifier covers forged authority, stale evidence, replay, response correlation, and branch scope.

### R01 — `internalization.repowise-record`

- **Owner / boundary:** dependency-expert; owns migration of Repowise into one C03 record, not the record engine or adapter.
- **Dependencies:** C03, B01.
- **Affected files (initial):** new Repowise record under `orchestration/internalization/`, NOTICE projection, evidence fixtures.
- **Acceptance criteria:** record covers pinned source/version, license, index/docs boundaries, conformance results, measured value/cost, quota/security/maintenance evidence, update cadence, rollback, exit, and retirement triggers.
- **Smallest proving test:** validate the record and trace each declared production operation to B01 conformance evidence.
- **Compatibility / rollback:** audit record only; no provider upgrade or route change.
- **Ponytail Necessity Trace:** Repowise is a current production participant and needs evidence before C03 enforcement.
- **Economic implementation lane:** evidence collection and one data record; no code fork.
- **Independent verifier condition:** verifier checks record claims against current CLI routes/tests and rejects undocumented docs/index differences.

### R02 — `internalization.graph-record`

- **Owner / boundary:** dependency-expert; owns migration of internal/external graph providers into one C03 record, not graph implementation.
- **Dependencies:** C03, B02.
- **Affected files (initial):** graph-provider record, NOTICE/provenance projection, conformance evidence links.
- **Acceptance criteria:** record distinguishes internal and fallback implementations, compatibility, measured graph utility/cost, update sources, stale-snapshot risk, rollback, exit, and retirement criteria.
- **Smallest proving test:** validate record and prove both B02 implementations link to current conformance evidence.
- **Compatibility / rollback:** documentation/governance only; routes remain unchanged.
- **Ponytail Necessity Trace:** graph fallback is a current replaceability boundary requiring explicit lifecycle evidence.
- **Economic implementation lane:** one record referencing existing tests and revisions.
- **Independent verifier condition:** verifier confirms no external-provider claim is inferred from the internal implementation or vice versa.

### R03 — `internalization.sentrux-record`

- **Owner / boundary:** dependency-expert; owns Sentrux/shim internalization record migration, not normalization code.
- **Dependencies:** C03, B03.
- **Affected files (initial):** Sentrux record, license/provenance projection, conformance/measurement links.
- **Acceptance criteria:** record covers upstream and shim ownership, plugin/Windows compatibility, rule conformance, measured value/cost, security/update policy, rollback, exit, and shim retirement evidence.
- **Smallest proving test:** validate record and reject retirement readiness while upstream Windows/plugin conformance is missing.
- **Compatibility / rollback:** no shim deletion; record captures current blocked retirement state.
- **Ponytail Necessity Trace:** the retained shim has an explicit lifecycle debt in the Gain Ledger.
- **Economic implementation lane:** one record using existing tests and upstream evidence.
- **Independent verifier condition:** verifier checks every retained fallback claim and retirement blocker against fresh conformance results.

### R04 — `internalization.codenexus-record`

- **Owner / boundary:** dependency-expert; owns CodeNexus/lite record migration, not the CodeNexus adapter.
- **Dependencies:** C03, B04.
- **Affected files (initial):** CodeNexus record, provenance/NOTICE projection, adapter evidence links.
- **Acceptance criteria:** record defines external ownership, lite adoption rung, storage/process boundary, conformance and measured localization value, update path, rollback, exit, and lite retirement criteria.
- **Smallest proving test:** validate record and trace full/lite claims to B04 swap tests and measured fixtures.
- **Compatibility / rollback:** no worker revival or removal; record preserves current lite fallback.
- **Ponytail Necessity Trace:** CodeNexus is the named perception boundary and must not drift into implicit vendoring.
- **Economic implementation lane:** one evidence record; no source import.
- **Independent verifier condition:** verifier confirms Pipeline-owned claims stop at the port and all internalization is explicitly justified.

### R05 — `internalization.repomix-record`

- **Owner / boundary:** dependency-expert; owns Repomix production-use record migration, not registry reconciliation or invocation.
- **Dependencies:** C03, B07.
- **Affected files (initial):** Repomix record, provenance projection, package/version and measurement evidence.
- **Acceptance criteria:** record covers packaging capability, revision/license, output conformance, size/token measurements, security/update policy, rollback, alternatives, exit, and retirement conditions.
- **Smallest proving test:** validate record and link the production invocation registered by B07 to conformance and measurement evidence.
- **Compatibility / rollback:** no invocation change; B07 may instead delete unused production participation.
- **Ponytail Necessity Trace:** register-or-delete audit identifies Repomix as a production participant requiring lifecycle evidence if retained.
- **Economic implementation lane:** retain invoke rung if justified; do not vendor.
- **Independent verifier condition:** verifier confirms measured benefit exceeds existing inventory/artifact alternatives and checks license/version facts.

### R06 — `internalization.native-code-evidence-record`

- **Owner / boundary:** dependency-expert; owns Native Code Evidence capability record migration, not its implementation.
- **Dependencies:** C03, B07.
- **Affected files (initial):** Native Code Evidence record, provenance projection, conformance/benchmark evidence.
- **Acceptance criteria:** record states owned versus borrowed semantics, implementation revision, artifact conformance, measured precision/cost, update policy, rollback, exit, and retirement conditions.
- **Smallest proving test:** validate record and trace the B07 production declaration to current code-evidence A/B and contract tests.
- **Compatibility / rollback:** record-only change; undeclared/dead implementation may be deleted by B07 instead.
- **Ponytail Necessity Trace:** a production evidence source needs explicit ownership and measurement, even when internal.
- **Economic implementation lane:** reuse existing benchmarks and record facts; no new analyzer.
- **Independent verifier condition:** verifier separates native capability evidence from external cocoindex claims and audits measurements.

### R07 — `internalization.cocoindex-record`

- **Owner / boundary:** dependency-expert; owns cocoindex adoption/rejection record migration, not code-evidence implementation.
- **Dependencies:** C03, B07.
- **Affected files (initial):** cocoindex record or reviewed retirement record, provenance/NOTICE projection, comparison evidence.
- **Acceptance criteria:** record declares actual production status, revision/license, conformance, measured incremental value/cost versus Native Code Evidence, security/update policy, rollback, exit, and retirement evidence; if unused, B07 deletion evidence closes it.
- **Smallest proving test:** validator rejects “available in repository” as adoption evidence and requires either production conformance/measurement or a reviewed retirement record.
- **Compatibility / rollback:** no automatic enablement; restore only through a new authority-approved record state.
- **Ponytail Necessity Trace:** prevents dormant reference code from masquerading as a maintained capability.
- **Economic implementation lane:** prefer delete/retire unless measured unique value exists.
- **Independent verifier condition:** verifier reproduces the comparison or confirms all production invocation was removed.

### R08 — `internalization.github-research-record`

- **Owner / boundary:** dependency-expert; owns GitHub Solution Research record migration, not network search or adoption decisions.
- **Dependencies:** C03, B07.
- **Affected files (initial):** GitHub research record, network/provenance policy, conformance/measurement evidence links.
- **Acceptance criteria:** record covers API/tool revision, license/quotation obligations, network and credential effects, dossier conformance, measured blocker-resolution value/cost, update policy, rollback, exit, and retirement conditions.
- **Smallest proving test:** validate a recorded research run and reject one lacking source revision, network effect, or measurement evidence.
- **Compatibility / rollback:** current opt-out flag remains; no external write authority is added.
- **Ponytail Necessity Trace:** network research is a production participant with cost, provenance, and supply-chain obligations.
- **Economic implementation lane:** keep read-only invoke/adapt rung; no crawler platform.
- **Independent verifier condition:** verifier checks source attribution, network effects, credentials redaction, reproducibility limits, and retirement criteria.

### R09 — `internalization.rg-record`

- **Owner / boundary:** dependency-expert; owns the ripgrep executable/reference lifecycle record, not inventory implementation.
- **Dependencies:** C03, A01.
- **Affected files (initial):** rg record, license/version/provenance evidence, inventory conformance and benchmark links.
- **Acceptance criteria:** records revision/license, CLI contract, platform availability, scope/exclusion conformance, measured inventory speed/cost, security/update cadence, replacement adapter, rollback, exit, and retirement criteria.
- **Smallest proving test:** validate the record and trace `inventory.rg` parity/benchmark evidence to a pinned executable identity.
- **Compatibility / rollback:** no rg upgrade; alternate inventory executable requires a new approved record state.
- **Ponytail Necessity Trace:** rg is an active-required external executable and the first real envelope implementation.
- **Economic implementation lane:** keep invoke rung; do not vendor or reimplement exact search.
- **Independent verifier condition:** verifier checks installed/source identity, license, cross-platform behavior, scope failures, measurement, and tested replacement command.

### R10 — `internalization.git-record`

- **Owner / boundary:** dependency-expert; owns the Git executable/protocol lifecycle record, not Snapshot Identity semantics.
- **Dependencies:** C03, A02, B07.
- **Affected files (initial):** Git record, license/version/provenance evidence, read-only command conformance and exit tests.
- **Acceptance criteria:** records revision/license, read-only operations, working-tree/HEAD semantics, measured cost, security/update policy, alternate-VCS port, rollback, exit, and retirement/out-of-scope conditions for mutation commands.
- **Smallest proving test:** validate the record and prove all current Git production calls are registered read-only effects or authority-gated mutations.
- **Compatibility / rollback:** no Git upgrade or mutation authority; direct calls remain until registry retirement tickets exist.
- **Ponytail Necessity Trace:** Git is drift-unregistered but foundational to snapshot and history evidence.
- **Economic implementation lane:** centralize invoke semantics; no Git library replacement.
- **Independent verifier condition:** verifier traces command call sites, dirty/HEAD behavior, license/version, mutation exclusion, and alternate-provider exit.

### R11 — `internalization.tree-sitter-v-record`

- **Owner / boundary:** dependency-expert; owns the V grammar/overlay record, not Sentrux parsing.
- **Dependencies:** C03, B03.
- **Affected files (initial):** tree-sitter-v record, overlay metadata, source revision/ABI/license evidence, plugin conformance tests.
- **Acceptance criteria:** records pinned grammar revision, MIT obligations, compiled artifact digest/ABI, Sentrux conformance, measured value/cost, security/update rebuild procedure, rollback, upstream replacement, and overlay retirement proof.
- **Smallest proving test:** reject the current URL/license-only evidence until revision and compiled digest are bound; accept a pinned reproducible fixture.
- **Compatibility / rollback:** retain overlay until upstream plugin passes the same tests.
- **Ponytail Necessity Trace:** the map identifies this as the only partially attributed optional parser and an explicit retirement dependency.
- **Economic implementation lane:** pin/rebuild existing overlay; do not fork grammar.
- **Independent verifier condition:** verifier rebuilds or verifies digest/ABI, license, V fixtures, upstream comparison, and rollback.

### R12 — `internalization.greenfield-record`

- **Owner / boundary:** dependency-expert; owns the Greenfield provider/reference record, not specification authority.
- **Dependencies:** C03, B07.
- **Affected files (initial):** Greenfield record, provider contract/provenance evidence, conformance/measurement fixtures.
- **Acceptance criteria:** records plugin revision/license, interactive/noninteractive boundary, artifact conformance, measured specification value/cost, security/update policy, review authority, rollback, exit, and plan-only retirement option.
- **Smallest proving test:** reject generated specs without plugin/source provenance and review status; accept a conforming plan-only fixture.
- **Compatibility / rollback:** default scan remains nonblocking; no plugin auto-execution.
- **Ponytail Necessity Trace:** Greenfield is an active-optional external provider in the manifest.
- **Economic implementation lane:** preserve provider-contract/plan-only rung; do not internalize plugin runtime.
- **Independent verifier condition:** verifier checks interactivity, provenance, license, no-auto-authority, conformance, measurement, and removal path.

### R13 — `internalization.openspec-record`

- **Owner / boundary:** dependency-expert; owns the OpenSpec reference/candidate lifecycle record, not workflow recommendation.
- **Dependencies:** C03, B06.
- **Affected files (initial):** OpenSpec record, absorbed-method evidence, candidate conformance/measurement and retirement evidence.
- **Acceptance criteria:** records revision/license, absorbed versus external semantics, recommendation conformance, measured value/cost, update policy, no-auto-init boundary, rollback, replacement, and retirement/out-of-scope state.
- **Smallest proving test:** validate an OpenSpec record and reject recommendation enablement without source revision and no-auto-init conformance.
- **Compatibility / rollback:** candidate can be removed from data without changing recommender authority.
- **Ponytail Necessity Trace:** OpenSpec is an explicit current advisory reference embedded in duplicate logic.
- **Economic implementation lane:** data candidate/reference only; no runtime dependency.
- **Independent verifier condition:** verifier checks license/revision, absorbed semantics, recommendation evidence, measurement, and tested removal.

### R14 — `internalization.spec-kit-record`

- **Owner / boundary:** dependency-expert; owns the spec-kit reference/candidate record, not method selection.
- **Dependencies:** C03, B06.
- **Affected files (initial):** spec-kit record, source/provenance, candidate conformance/measurement and exit evidence.
- **Acceptance criteria:** records revision/license, greenfield fit claims, conformance, measured value/cost, update policy, no-auto-init boundary, rollback, replacement, and retirement/out-of-scope state.
- **Smallest proving test:** reject a fit claim based only on tool name; accept a pinned, measured, removable candidate record.
- **Compatibility / rollback:** advisory candidate only; removal cannot change facts or commitments.
- **Ponytail Necessity Trace:** spec-kit is a named active recommendation reference.
- **Economic implementation lane:** keep reference/data rung; no installation.
- **Independent verifier condition:** verifier checks evidence for fit, revision/license, measurement, authority boundary, and removal.

### R15 — `internalization.matt-flow-record`

- **Owner / boundary:** dependency-expert; owns the matt-flow reference/candidate record, not recommender rules.
- **Dependencies:** C03, B06.
- **Affected files (initial):** matt-flow record, provenance/license, absorbed-concept conformance/measurement and exit evidence.
- **Acceptance criteria:** records exact source revision/license, internalized concepts, candidate behavior, measured value/cost, update policy, rollback, replacement, and retirement/out-of-scope state.
- **Smallest proving test:** validate source-to-internal-concept trace and reject unversioned or unmeasured retention.
- **Compatibility / rollback:** remove candidate/reference without changing stable advisory schema.
- **Ponytail Necessity Trace:** matt-flow is named in the embedded three-stack recommender.
- **Economic implementation lane:** internalized data/rules only when justified; no external runtime.
- **Independent verifier condition:** verifier checks source trace, license, semantics, measurement, update and removal evidence.

### R16 — `internalization.gstack-record`

- **Owner / boundary:** dependency-expert; owns the gstack reference/candidate record, not recommender execution.
- **Dependencies:** C03, B06.
- **Affected files (initial):** gstack record, provenance/license, candidate conformance/measurement and exit evidence.
- **Acceptance criteria:** records revision/license, relevant workflow semantics, conformance, measured value/cost, update policy, no execution authority, rollback, replacement, and retirement/out-of-scope state.
- **Smallest proving test:** reject an unpinned gstack recommendation source and accept a removable evidence-bound candidate.
- **Compatibility / rollback:** advisory-only data; removal leaves facts and plan authority unchanged.
- **Ponytail Necessity Trace:** gstack is a named current advisory reference.
- **Economic implementation lane:** reference/data rung only.
- **Independent verifier condition:** verifier checks provenance, license, conformance, measurement, authority isolation, and deletion path.

### R17 — `internalization.qiaomu-goal-record`

- **Owner / boundary:** dependency-expert; owns the qiaomu-goal-meta-skill design-reference record, not Agent Goal Intake runtime.
- **Dependencies:** C03.
- **Affected files (initial):** qiaomu reference record, absorbed goal-contract trace, benchmark/update/retirement evidence.
- **Acceptance criteria:** records revision/license, absorbed semantics, conformance to Agent Goal Intake, measured value, update review, rollback, replacement, and authority-approved reference-only/retirement state.
- **Smallest proving test:** validate source-to-contract trace and reject scanner runtime dependency or unapproved retention.
- **Compatibility / rollback:** reference may be retired while internal task contract remains versioned.
- **Ponytail Necessity Trace:** map marks it reference-internalized and therefore requires lifecycle evidence, not manifest registration.
- **Economic implementation lane:** evidence record only; no runtime adoption.
- **Independent verifier condition:** verifier checks source/license, semantic trace, non-runtime boundary, measurement, and removal.

### R18 — `internalization.agent-loops-record`

- **Owner / boundary:** dependency-expert; owns awesome-agent-loops design-reference record, not loop execution.
- **Dependencies:** C03.
- **Affected files (initial):** agent-loops record, absorbed pattern trace, conformance/measurement/update/exit evidence.
- **Acceptance criteria:** records revision/license, chosen absorbed loop semantics, measured utility, update policy, scanner isolation, rollback, replacement, and retirement/out-of-scope decision.
- **Smallest proving test:** reject an unpinned catalog reference and prove removal does not change scanner contracts.
- **Compatibility / rollback:** internal loop vocabulary remains; reference can be retired independently.
- **Ponytail Necessity Trace:** it is a separate upstream reference in the combined map row.
- **Economic implementation lane:** reference evidence only.
- **Independent verifier condition:** verifier checks attribution, semantic trace, non-runtime coupling, measurement, and tested exit.

### R19 — `internalization.metaharness-record`

- **Owner / boundary:** dependency-expert; owns MetaHarness reference-only lifecycle decision, not distribution implementation.
- **Dependencies:** C03.
- **Affected files (initial):** MetaHarness record, harness-reference provenance, authority decision and retirement evidence.
- **Acceptance criteria:** records revision/license, reference-only boundary, measured design value, update review, replacement options, rollback, and explicit authority-approved retain/retire/out-of-scope state.
- **Smallest proving test:** reject indefinite reference-only status without authority/expiry; accept a reviewed out-of-runtime decision.
- **Compatibility / rollback:** no runtime effect; reference can be removed without scanner changes.
- **Ponytail Necessity Trace:** reference-only material still consumes maintenance/decision attention.
- **Economic implementation lane:** record and authority decision only.
- **Independent verifier condition:** verifier checks no runtime invocation, source/license, decision authority, expiry, and removal proof.

### R20 — `internalization.yao-meta-skill-record`

- **Owner / boundary:** dependency-expert; owns yao-meta-skill benchmark-reference record, not the internal benchmark gate.
- **Dependencies:** C03.
- **Affected files (initial):** yao record, benchmark semantic trace, measurement/update/retirement evidence.
- **Acceptance criteria:** records revision/license, absorbed benchmark criteria, conformance and measured usefulness, update review, rollback, replacement, and retirement state; local doc test is not upstream execution proof.
- **Smallest proving test:** reject a record citing only `scripts/tests/test-skill-development-benchmark.ps1` pass; require pinned source and semantic conformance.
- **Compatibility / rollback:** internal benchmark remains if reference is retired.
- **Ponytail Necessity Trace:** map marks the source reference-internalized but current evidence is terminological.
- **Economic implementation lane:** record existing absorbed criteria; no upstream runtime.
- **Independent verifier condition:** verifier checks source/license, semantic mapping, meaningful measurement, update and independent removal.

### R21 — `internalization.ponytail-record`

- **Owner / boundary:** dependency-expert; owns Ponytail source-reference lifecycle record, not C00 enforcement.
- **Dependencies:** C03, C00.
- **Affected files (initial):** Ponytail record, policy-source trace, C00 conformance/measurement/update/retirement evidence.
- **Acceptance criteria:** records revision/license, absorbed Value Filter/Necessity Trace semantics, C00 conformance, measured rejection/overhead, update policy, rollback, replacement, and source-reference retirement state.
- **Smallest proving test:** validate source-to-C00 behavior cases and reject terminology-only evidence.
- **Compatibility / rollback:** C00 semantic contract remains if source reference is retired.
- **Ponytail Necessity Trace:** the executable gate itself requires provenance and lifecycle discipline.
- **Economic implementation lane:** record and tests, no source vendoring.
- **Independent verifier condition:** verifier checks source/license, behavioral trace, safety exceptions, measurement, updates, and removable source coupling.

### R22 — `internalization.mattpocock-skills-record`

- **Owner / boundary:** dependency-expert; owns mattpocock/skills reference record, not project-management support.
- **Dependencies:** C03, B06.
- **Affected files (initial):** mattpocock record, absorbed issue/triage/domain/workflow trace, measurement/update/exit evidence.
- **Acceptance criteria:** records revision/license, each absorbed concept, conformance, measured value, update policy, no external-write authority, rollback, replacement, and retirement state.
- **Smallest proving test:** reject a record that cannot trace a retained concept or prove removal of an unused one.
- **Compatibility / rollback:** internal project-management contracts survive source retirement.
- **Ponytail Necessity Trace:** map marks this reference-internalized across docs and recommender.
- **Economic implementation lane:** retain only evidenced concepts, delete unused references.
- **Independent verifier condition:** verifier checks provenance/license, concept trace, external-effect boundary, measurement, and exit.

### R23 — `internalization.linear-record`

- **Owner / boundary:** dependency-expert; owns Linear reference-only/out-of-scope decision record, not tracker integration.
- **Dependencies:** C03, A05.
- **Affected files (initial):** Linear record, authority-approved scope decision, credential/external-effect and retirement evidence.
- **Acceptance criteria:** records source/API/license terms as applicable, optional projection boundary, measurement or explicit no-current-use, update/credential policy, rollback, replacement, and authority-approved retain/retire/out-of-scope state with expiry.
- **Smallest proving test:** reject implicit in-scope status and accept a signed no-scanner-write/out-of-scope record that expires for review.
- **Compatibility / rollback:** no connector install or external mutation is authorized.
- **Ponytail Necessity Trace:** reference-only hosted state needs an explicit decision to avoid phantom scope.
- **Economic implementation lane:** out-of-scope record unless an approved project authority requires projection.
- **Independent verifier condition:** verifier checks authority, expiry, credentials, external effects, single-authority rule, and removal.

### R24 — `internalization.obsidian-record`

- **Owner / boundary:** dependency-expert; owns Obsidian reference-only projection record, not wiki implementation.
- **Dependencies:** C03, A05.
- **Affected files (initial):** Obsidian record, authority/scope decision, projection conformance/measurement/retirement evidence.
- **Acceptance criteria:** records reference/version/license context, view-only or explicit task-authority boundary, measurement/no-use evidence, update policy, rollback, replacement, and authority-approved retain/retire/out-of-scope state.
- **Smallest proving test:** reject a record allowing Obsidian to replace scanner artifacts; accept an expiring view-only out-of-scope decision.
- **Compatibility / rollback:** repo artifacts remain authority; no projection is required.
- **Ponytail Necessity Trace:** optional knowledge surfaces must not become undocumented product scope.
- **Economic implementation lane:** record-only unless separately approved.
- **Independent verifier condition:** verifier checks authority isolation, no runtime invocation, measurement/scope evidence, expiry, and removal.

### R25 — `internalization.llm-wiki-record`

- **Owner / boundary:** dependency-expert; owns the LLM Wiki reference-only projection record, not LLM generation.
- **Dependencies:** C03, A05.
- **Affected files (initial):** LLM Wiki record, authority/scope and model/provider provenance policy, retirement evidence.
- **Acceptance criteria:** records reference/provider assumptions, non-authoritative view boundary, measurement/no-use evidence, model/update policy, data/security effects, rollback, replacement, and authority-approved retain/retire/out-of-scope state.
- **Smallest proving test:** reject any record promoting generated wiki text to Engineering Fact; accept a view-only expiring scope decision.
- **Compatibility / rollback:** no model/provider runtime is required by Pipeline.
- **Ponytail Necessity Trace:** separates this external reference from Obsidian and prevents implicit LLM dependency.
- **Economic implementation lane:** record/out-of-scope decision only.
- **Independent verifier condition:** verifier checks model/data effects, authority boundary, expiry, no runtime coupling, and deletion path.

### R26 — `internalization.my-code-machine-record`

- **Owner / boundary:** dependency-expert/architect; owns the my-code-machine proposed-reference/adoption decision record, not host mutation or migration.
- **Dependencies:** C03, A05.
- **Affected files (initial):** my-code-machine record, ADR reconciliation, capability comparison/measurement, authority and retirement evidence.
- **Acceptance criteria:** records revision/license, proposed host capability boundary, conformance gaps, measured value/cost, mutation/security effects, update policy, rollback, exit, and authority-approved adopt/defer/retire/out-of-scope decision consistent with no-big-bang ADR 0010.
- **Smallest proving test:** reject a merge/migration record without atom parity, effect authority, measurement, and rollback; accept a reviewed defer/out-of-scope state.
- **Compatibility / rollback:** current external tool remains separate; no merge or rewrite is authorized.
- **Ponytail Necessity Trace:** map marks a high-scope proposed merge requiring explicit closure rather than perpetual ambiguity.
- **Economic implementation lane:** decision record first; integration only through later atomic tickets if approved.
- **Independent verifier condition:** verifier checks source/license, scope, effect risk, ADR consistency, measurement, authority, and tested exit/defer semantics.

### D01 — `project.orientation`

- **Owner / boundary:** executor; owns the first actionable deterministic project view, not deep architecture analysis.
- **Dependencies:** A02, A03, B05, B08.
- **Affected files (initial):** orientation schema/atom, facade summary projection, fixtures/tests.
- **Acceptance criteria:** reports identity, purpose evidence, languages, boundaries, entry points, commands, active change, evidence availability, risks, unknowns, and confidence; works without LLM.
- **Smallest proving test:** fixture repo produces schema-valid orientation with explicit unknown purpose when evidence is absent.
- **Compatibility / rollback:** existing summary remains; new orientation is additive until benchmark passes.
- **Ponytail Necessity Trace:** shortest useful understanding view promised by ADR 0010.
- **Economic implementation lane:** compose existing inventory/Git/provider facts; no semantic model.
- **Independent verifier condition:** verifier checks every claim has provenance and missing evidence remains unknown.

### D02 — `project.orientation-benchmark`

- **Owner / boundary:** test-engineer; owns latency/quality measurement, not orientation generation.
- **Dependencies:** D01.
- **Affected files (initial):** benchmark runner, representative fixture corpus, CI job/report schema.
- **Acceptance criteria:** measures p50/p95 wall time, cold/warm runs, field correctness, unknown precision, and provenance completeness; typical fixtures meet the 60-second no-LLM target; failures identify cost center.
- **Smallest proving test:** run the representative corpus across declared small/medium/large and clean/dirty/provider-missing classes with repeated cold/warm samples; prove reported p50 and p95 satisfy the documented 60-second target for the “typical” corpus stratum while quality/provenance thresholds pass, and reject a provenance-free fast result.
- **Compatibility / rollback:** non-blocking CI until stable, then gate documented corpus.
- **Ponytail Necessity Trace:** prevents “fast understanding” from remaining an unmeasured slogan.
- **Economic implementation lane:** existing test infrastructure and local fixtures; no hosted benchmark service.
- **Independent verifier condition:** verifier repeats on a clean machine and audits corpus representativeness and timing methodology.

### D03 — `understanding.quadrant`

- **Owner / boundary:** architect/executor; owns classification into Known Core, Critical Unknown, Supporting Context, Deferred Unknown, not fact generation.
- **Dependencies:** D01, C01.
- **Affected files (initial):** quadrant model/schema, rules/tests, orientation projection.
- **Acceptance criteria:** classification derives from system criticality and evidence confidence; unknowns remain visible; method/card selection may consume but not rewrite it.
- **Smallest proving test:** critical low-confidence dependency becomes Critical Unknown while low-criticality low-confidence material is Deferred Unknown.
- **Compatibility / rollback:** additive derived model.
- **Ponytail Necessity Trace:** focuses scarce analysis effort without full-repository over-reading.
- **Economic implementation lane:** deterministic matrix, no ML.
- **Independent verifier condition:** verifier checks boundary values, provenance, and stable classification.

### D04 — `delivery.light-speed-measure`

- **Owner / boundary:** test-engineer; owns measurement of avoidable wait/handoff/rework after preserving required gates, not schedule commitments.
- **Dependencies:** A07, D02, C01.
- **Affected files (initial):** run timing/event schema, measurement atom, benchmark report/tests.
- **Acceptance criteria:** separates irreducible technical work from queue, handoff, repeated understanding, rework, and unnecessary coordination; reports baseline/delta without rewarding skipped verification.
- **Smallest proving test:** synthetic trace attributes queue delay but does not count mandatory test time as waste.
- **Compatibility / rollback:** telemetry is opt-in and local; no behavior gate initially.
- **Ponytail Necessity Trace:** makes the Light-Speed promise falsifiable and economically actionable.
- **Economic implementation lane:** consume existing run events; no observability platform.
- **Independent verifier condition:** verifier audits attribution rules against value-stream mapping, queueing, and critical-path principles.

### E00 — `compatibility.retirement-gate`

- **Owner / boundary:** verifier/executor pair; owns the executable decision that one legacy branch is safe to retire, not branch deletion or language migration.
- **Dependencies:** A01, A07, A08, B04, B06, B07, C00, D02.
- **Affected files (initial):** retirement-manifest schema, gate module/tests, `docs/ponytail-gain-ledger.md` projection.
- **Acceptance criteria:** gate requires replacement atom, golden parity, contract/effect parity, production registry reconciliation, compatibility window evidence, owner, rollback command/test, usage observation, and independent approval; unproven or cyclic replacement fails; line reduction is never correctness evidence.
- **Smallest proving test:** submit a retirement record missing rollback execution evidence and assert failure, then accept a fully evidenced synthetic branch.
- **Compatibility / rollback:** gate is additive and cannot delete code; disabling it prevents new retirement approvals but preserves records.
- **Ponytail Necessity Trace:** one reusable gate prevents subjective, line-count-driven PowerShell deletion.
- **Economic implementation lane:** schema plus deterministic validator over existing test artifacts; no migration platform.
- **Independent verifier condition:** verifier authors adversarial manifests and confirms every required evidence class is independently checked and content-bound.

### E01 — `compatibility.retirement-ticket-template`

- **Owner / boundary:** planner; owns the executable per-branch retirement manifest template and completeness lint, not approval or deletion.
- **Dependencies:** E00.
- **Affected files (initial):** retirement ticket template/schema examples, lint test, contributor guidance.
- **Acceptance criteria:** template identifies exactly one legacy branch/call path, replacement capability, dependencies, affected files, parity/effect/usage evidence, rollback rehearsal, deletion diff, verifier, and observation expiry; multi-branch entries fail lint.
- **Smallest proving test:** lint one single-branch fixture successfully and reject a fixture containing recommender plus provider-preflight deletion.
- **Compatibility / rollback:** template is additive; branch tickets remain independent artifacts.
- **Ponytail Necessity Trace:** enforces small reversible retirements instead of a facade-sized change.
- **Economic implementation lane:** data template and test only.
- **Independent verifier condition:** verifier creates ambiguous, multi-branch, expired-evidence, and missing-owner examples and confirms rejection.

### E02 — `compatibility.retire-recommender-branch`

- **Owner / boundary:** executor; owns deletion of the duplicated workflow-recommender branch in `run-code-intel.ps1`, not recommender behavior.
- **Dependencies:** E01, B06.
- **Affected files (initial):** `run-code-intel.ps1`, `OpenSpec-Detector.ps1` adapter path, recommender parity tests, one retirement record.
- **Acceptance criteria:** main-runner duplicate is absent; legacy flags route to B06; A00 parity, effects, no-auto-init, rollback rehearsal, and E00 approval pass.
- **Smallest proving test:** facade and B06 return normalized parity for all recommender fixtures after the duplicate block is deleted.
- **Compatibility / rollback:** restore the prior adapter branch/tag during the bounded window without changing B06.
- **Ponytail Necessity Trace:** removes a proven duplicate named in the commitment ledger.
- **Economic implementation lane:** deletion plus flag mapping; no algorithm rewrite.
- **Independent verifier condition:** verifier reruns recommender and facade suites and executes rollback on Windows.

### E03 — `compatibility.retire-provider-preflight-branch`

- **Owner / boundary:** executor; owns removal of direct production invocation of `scripts/tests/test-code-intel-provider.ps1`, not Repowise behavior.
- **Dependencies:** E01, B01.
- **Affected files (initial):** `run-code-intel.ps1`, Repowise facade route, provider tests, one retirement record.
- **Acceptance criteria:** production routes through B01/A04; the test script is test-only or renamed appropriately; quota/index-only parity, effects, rollback rehearsal, and E00 approval pass.
- **Smallest proving test:** provider quota fixture preserves index-only behavior through B01 with no production call to a `test-*` file.
- **Compatibility / rollback:** legacy probe remains callable diagnostically during the observation window.
- **Ponytail Necessity Trace:** restores the test/production boundary and removes embedded provider-specific orchestration.
- **Economic implementation lane:** route replacement and deletion only.
- **Independent verifier condition:** verifier statically proves no production call remains and reruns provider quota/missing-tool cases plus rollback.

### E04 — `compatibility.retire-codenexus-direct-branch`

- **Owner / boundary:** executor; owns removal of direct CodeNexus-lite invocation from the facade, not CodeNexus/lite implementation.
- **Dependencies:** E01, B04, B05.
- **Affected files (initial):** `run-code-intel.ps1`, CodeNexus adapter route, localization/survival tests, one retirement record.
- **Acceptance criteria:** facade invokes B04; unavailable provider selects B05 without structural overclaim; parity/effects/provider-swap/rollback evidence and E00 approval pass.
- **Smallest proving test:** full, lite, and unavailable fixtures traverse the port with expected parity and no direct script call in facade.
- **Compatibility / rollback:** legacy direct call remains tagged for bounded rollback only.
- **Ponytail Necessity Trace:** realizes the promised CodeNexus ownership boundary without duplicating perception.
- **Economic implementation lane:** call-site substitution after conformance, no worker rewrite.
- **Independent verifier condition:** verifier audits process/storage decoupling, fallback claims, facade call graph, and rollback.

### E05 — `compatibility.retire-publication-branch`

- **Owner / boundary:** executor; owns removal of the legacy staging/promotion/completion-marker branch from `run-code-intel.ps1`, not index traversal or artifact generation.
- **Dependencies:** E01, A07, A09.
- **Affected files (initial):** `run-code-intel.ps1`, publication facade adapter, transactional publication tests, one retirement record.
- **Acceptance criteria:** facade routes publication through A09→A07; the current partial dirty-worktree staging/marker code is removed only after interruption/effect/parity/rollback evidence and E00 approval; index implementation is untouched by this ticket.
- **Smallest proving test:** inject failure at every A07 phase through the facade and assert no completed final run, then complete and assert marker-last publication with A00 parity.
- **Compatibility / rollback:** explicit legacy publication route remains during the bounded window and is isolated from committed-only index authority.
- **Ponytail Necessity Trace:** retires exactly one legacy publication branch after the atomic publisher is proven.
- **Economic implementation lane:** route replacement and deletion only; reuse the current draft test as partial regression input, not completion proof.
- **Independent verifier condition:** verifier runs the publication interruption matrix, effects/parity, static branch audit, and rollback on Windows without evaluating index retirement.

### E07 — `compatibility.retire-native-code-branch`

- **Owner / boundary:** executor; owns removal of embedded Native Code Evidence production functions/call path from the facade, not B08 behavior.
- **Dependencies:** E01, B08, B07.
- **Affected files (initial):** `run-code-intel.ps1`, B08 adapter route, code-evidence tests, one retirement record.
- **Acceptance criteria:** normal/full modes invoke B08 through A09; embedded production branch is absent; artifact/effect/parity/unsupported-language/rollback evidence and E00 approval pass.
- **Smallest proving test:** public mode matrix produces normalized Code Evidence parity through B08 while static audit finds no embedded production invocation.
- **Compatibility / rollback:** prior embedded adapter remains tagged for the observation window.
- **Ponytail Necessity Trace:** removes an active drift-unregistered monolith branch only after the native atom exists.
- **Economic implementation lane:** call-site substitution and deletion; no algorithm rewrite.
- **Independent verifier condition:** verifier reruns Code Evidence/A-B suites, checks registry path, unsupported claims, and rollback.

### E08 — `compatibility.retire-hospital-branch`

- **Owner / boundary:** executor; owns removal of embedded Hospital diagnosis/rendering production call path, not B09 diagnosis semantics.
- **Dependencies:** E01, B09, B07.
- **Affected files (initial):** `run-code-intel.ps1`, B09 adapter route, hospital parity/precedence/rendering tests, one retirement record.
- **Acceptance criteria:** facade consumes B09 machine result and rebuildable view; embedded diagnosis branch is absent; fail-closed precedence/parity/effects/rollback evidence and E00 approval pass.
- **Smallest proving test:** public fixture with untrusted authoritative evidence yields identical machine diagnosis through B09 and static audit finds no embedded decision path.
- **Compatibility / rollback:** old Hospital adapter remains tagged for bounded rollback and cannot override B09 in normal mode.
- **Ponytail Necessity Trace:** realizes the registered `diagnosis.hospital` atom instead of merely naming it in the manifest.
- **Economic implementation lane:** extract/route/delete existing pure logic; no rules engine.
- **Independent verifier condition:** verifier reruns diagnosis matrix, view rebuild, registry route, static deletion, and rollback.

### E09 — `compatibility.retire-doctor-wrapper-branch`

- **Owner / boundary:** executor; owns removal of the non-enveloped production doctor routing branch while retaining necessary bootstrap shell glue, not B10 probe semantics.
- **Dependencies:** E01, B10, B07.
- **Affected files (initial):** `invoke-code-intel.ps1`, `check-code-intel-tools.ps1`, Rust doctor adapter route, doctor tests, one retirement record.
- **Acceptance criteria:** public doctor/run preflight routes through B10 envelopes; any retained bootstrap branch is explicitly non-authoritative, registered, owned, and expiring; readiness/conformance separation, secret redaction, parity, rollback, and E00 approval pass.
- **Smallest proving test:** run public preflight with manifest drift and present-but-nonconforming provider; assert one B10 result, no secret, correct readiness diagnosis, and no unowned direct production route.
- **Compatibility / rollback:** minimal shell bootstrap remains if required for locating the binary; prior routing is tagged for bounded rollback.
- **Ponytail Necessity Trace:** closes the active-required doctor gap without deleting indispensable fresh-machine bootstrap behavior.
- **Economic implementation lane:** wrapper route change and deletion of redundant probes only.
- **Independent verifier condition:** verifier tests bootstrap/fresh-machine path, stdout purity, manifest/provider cases, static route ownership, and rollback.

### E10 — `compatibility.retire-index-branch`

- **Owner / boundary:** executor; owns removal of the legacy `update-code-intel-index.ps1` production traversal/call path, not publication or A08 index semantics.
- **Dependencies:** E01, A08, E05.
- **Affected files (initial):** `update-code-intel-index.ps1`, public wrapper/index route, index tests, one retirement record.
- **Acceptance criteria:** public index refresh routes through A08; current partial dirty-worktree staging/marker guard is retained as regression evidence until replaced; legacy production traversal is absent; rebuild/parity/forged-marker/rollback evidence and E00 approval pass.
- **Smallest proving test:** rebuild an index containing staged, markerless, forged, and valid runs through A08 and assert only the valid run appears, then statically prove no public production call reaches the legacy traversal.
- **Compatibility / rollback:** old script remains tagged/diagnostic during the bounded observation window and cannot write the authoritative index in normal mode.
- **Ponytail Necessity Trace:** retires exactly the index branch after publication has independently converged.
- **Economic implementation lane:** route replacement and script demotion/deletion; no database.
- **Independent verifier condition:** verifier runs rebuild/incremental parity, adversarial marker cases, public call graph, and rollback without reopening E05 publication approval.

### E06 — `compatibility.facade-finalize`

- **Owner / boundary:** verifier; owns final determination that public PowerShell contains only supported compatibility/platform glue or may be retired, not remaining atom implementations.
- **Dependencies:** E02, E03, E04, E05, E07, E08, E09, E10.
- **Affected files (initial):** facade, stable wrapper/orchestrator entrypoints, final retirement manifest, README/installation command projections.
- **Acceptance criteria:** every remaining branch is registry-backed and has an owner/expiry or a completed single-branch retirement record; doctor, snapshot/inventory, provider adapters, Native Code Evidence, Hospital diagnosis, publication, and index all execute in the declared A09 DAG; supported modes pass A00 parity and rollback windows remain available; retained PowerShell is enumerated rather than assumed zero.
- **Smallest proving test:** run doctor plus lite/normal/full public mode matrix and assert every execution node is registry-backed, enveloped, admitted where applicable, committed, and indexed; static audit finds no unowned branch.
- **Compatibility / rollback:** public command remains a thin shim if installers/users require it; final deletion requires a separate explicit distribution decision.
- **Ponytail Necessity Trace:** final gate is reachable through independently proven single-branch retirements while preserving necessary platform glue.
- **Economic implementation lane:** delete or retain based on evidence; no forced language purity target.
- **Independent verifier condition:** verifier did not author E02-E05/E07-E10, reruns full DAG/mode/parity/rollback matrix, and signs only with zero unsupported branches.

## Execution waves and parallelism

| Wave | Tickets | Parallel rule | Exit evidence |
| --- | --- | --- | --- |
| 0 | A00 | solo | golden fixtures reproduce current behavior |
| 1 | A01 | solo | real `inventory.rg` executes through v1 envelopes with A00 parity |
| 2 | A02 | solo | portable snapshot identity negative/relocation tests pass |
| 3 | A03 | solo | Artifact Ref digest/snapshot/schema negative tests pass |
| 4 | A04, A06, A09 | parallel after A03 | provider-neutral admissibility, staged write, and executable DAG tests pass |
| 5 | A05, A07, B01, B02, B03, B04, B08 | parallel by dependency | authority gate, Run Commit, four provider conformance suites, and Native Code Evidence atom pass |
| 6 | A08, B05, B06, C00 | parallel by dependency | committed-only indexing, fallback boundaries, advisory authority, and executable Ponytail gate pass |
| 7 | B07, B09, C01, C03, C05, D01 | parallel by ownership | registry reconciliation, Hospital atom, method/internalization/gap/orientation contracts pass |
| 8 | B10, C02, C04, C06, D02, D03, R01-R26 | parallel after each named adapter/engine | enveloped doctor, method/discovery/decision port, representative corpus SLA, and all lifecycle/retirement records pass |
| 9 | C07, D04, E00 | parallel after prerequisites | durable decisions, Light-Speed baseline, and retirement gate pass |
| 10 | E01 | solo | single-branch retirement template/lint passes |
| 11 | E02, E03, E04, E05, E07, E08, E09 | parallel only in isolated files; serialize shared wrapper/facade edits | each single branch has independent parity, effects, rollback, and E00 approval |
| 12 | E10 | solo after E05 | index branch retires independently after publication convergence |
| 13 | E06 | solo independent approval | doctor plus public full-DAG mode matrix and zero-unsupported-branch audit pass |

## First five-ticket handoff

The next executor should take A00 only. After its verifier accepts the fixtures, execute A01, A02, A03, and A04 in that order. These first five tickets establish current parity, a real enveloped atom, snapshot identity, Artifact Ref verification, and provider-neutral admissibility. Do not start provider-specific adapter, recommender, or CodeNexus extraction before A04, because doing so would merely reproduce trust logic in each adapter.

For A00-A04, the pull-request stop condition is fresh targeted tests plus `cargo test`, affected PowerShell tests, schema validation, `git diff --check`, and an independent verifier report. A passing unit test alone is insufficient if the facade parity fixture differs.

## Principal risks

- **False parity:** normalization could erase meaningful provenance or verdict differences. Mitigation: verifier-owned fixture audit and deliberate mismatch tests.
- **Dual authority:** legacy reports and new authority artifacts may disagree during migration. Mitigation: audit-only phase, explicit precedence field, and fail-closed promotion.
- **Provider success confusion:** health/preflight success may still be treated as evidence admissibility. Mitigation: A04 is provider-neutral and B01-B04 keep native health/translation outside it.
- **CodeNexus duplication:** survival scanning could grow into a second graph product. Mitigation: B05 schema forbids structural/impact claims.
- **Method theater:** Method Cards may become documentation with no executable selection. Mitigation: C02 and proving fixtures are separate required capability work.
- **Big-bang retirement pressure:** line-count goals may encourage rewriting all PowerShell. Mitigation: E00/E01 enforce one branch per retirement record; E02-E05 and E07-E10 carry independent rollback before E06 can pass.
- **Benchmark gaming:** 60 seconds could be met by omitting unknowns or provenance. Mitigation: D02 gates quality and latency together.

## Final acceptance gate

ADR 0010 may be reported as implemented only when every ticket is implemented and independently verified; doctor, snapshot/inventory, provider evidence, Native Code Evidence, Hospital diagnosis, publication, and indexing execute through the full declared A09 DAG; A07/A08 interruption/index guarantees pass beyond the current partial dirty-worktree drafts; provider adapters pass the shared A04 conformance suite; B07 finds no undeclared production participant; C00 rejects unnecessary output; all R01-R26 records contain current conformance, measurement, update, and retirement evidence or an authority-approved expiring retirement/out-of-scope state; C06 is proven replaceable; the representative D02 corpus meets its p50/p95 quality-and-latency target; and E06 finds no unsupported facade branch. Remaining PowerShell must be documented compatibility or platform glue with an explicit owner. Until then, status reports must name completed ticket IDs and say the rest are planned or in progress.
