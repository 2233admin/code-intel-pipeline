# DR-0016: Versioned Quality admission, not a new Quality formula

Status: active
Date: 2026-10-10
Decision: For the owner-approved `evidence-quality-admission` policy v1, preserve DR-0011 all-scope Quality measurement and report `quality_degraded` as advisory; every other existing blocking rule and evidence boundary remains blocking.

## Authority and scope

The owner explicitly chose “接受 A，进入实施” after reviewing the protection loss in issue #427. Approval record: https://github.com/2233admin/code-intel-pipeline/issues/427#issuecomment-6095655906. The policy manifest is `orchestration/sentrux-gate-policy.v1.json`; its exact approved bytes have SHA-256 `f97cfed77d80be09acda1331bf449bb21e09ee58d91e19255313bbc5815d11f0`.

This decision authorizes implementation of those semantics. It does not attest the implementation, merge a failed candidate, authorize deployment, change baseline bytes, lower thresholds, or lift DR-0013. Policy identity is distinct from measurement identity. A candidate cannot choose an arbitrary policy from its target repository or authorize its own changed manifest: the implementation recognizes only the independently approved policy identity, and CI checks its bytes against the approval record outside the candidate tree.

## Why

Using one existing CI-compiled native engine, clean main `ff103df7` scored 6236, current-main overlays for #417/#418 scored 6235/6233, and historical combination `09baf25` scored 6232. Source-equivalent reconstruction of the same included bytes identified Equality/LOC distribution as the sole negative factor in those current-main comparisons; #418 included changes were all tests. Equal displayed factors alone were not used as proof. Unchanged heuristic edges are not proof of unchanged runtime dependencies or behavior.

The original v6 baseline declares source commit `5a6eb4422b90f9b12dfea36fa2e917ee01c9e064` / tree `1ae18ef3d5510abb9b210e1a3a22280ae183e53c`. A clean read-only checkout replayed with the same native engine exits 0 and reproduces all 12 stored numeric metrics, including 399 files, 5367 functions, Quality 6236, coupling 63.33, cycles 0 and God count 33. All 33 saved God paths, LOC/function/rule identities and exact declared-source content hashes were independently checked; the selected path sets match, not just their counts. Baseline bytes remain unchanged. Historical full-precision inputs and a contemporaneous dirty-tree content identity were not recorded: the replay attests the declared source and stored comparisons, not unrecorded original bytes.

## Deliberately accepted protection loss

The whole aggregate comparison is advisory, not only Equality. Modularity, depth, duplicate-only redundancy, and some multi-language graph regressions previously caught only by aggregate decline may no longer automatically block. The existing hard cycle scanner is Rust-only and is not an equivalent substitute for the Quality graph. This loss was disclosed and accepted; the policy must not be described as equivalent protection or a mathematical bug fix.

The alternative was to retain strict all-scope aggregate admission, or introduce a separately versioned production population, classifier and historical anchor. The latter is not authorized by this decision. Do not discard behavioral coverage or rearrange files to game a score; incidental wording tests may be removed under repository test policy. Do not add arbitrary tolerance, recompute a four-factor formula, or grant dependency/test-label exceptions.

## Enforcement

- DR-0011 formula/provider versions, all-scope file population, completeness limitations, v6 baseline and existing numerical thresholds do not change.
- Typed gate results contain approved policy identity, measurement and baseline identity, before/after comparisons, blocking violations and advisories. A Quality decrease remains visible and is never called “No degradation detected.”
- Unknown policy/provider/formula/scope/profile, invalid or missing baseline, stale/mismatched snapshot, unknown rules and missing required evidence fail closed for authoritative admission. A provider without the policy handshake cannot inherit native admission authority from exit 0 alone.
- CLI, DAG, provider/adapter, Artifact Ref validators, all current schemas and consumers, CI/release and installed artifacts must agree on policy identity. Historical contracts remain historical evidence, not an authority shortcut for new admission.
- CI compares the manifest digest with its independent approval record; editing the candidate manifest alone cannot approve another policy. Existing safety, behavioral, package, install-smoke and artifact-roundtrip checks are not skipped or converted to continue-on-error.
- Consumer-visible negative controls cover advisory-only decline, a true aggregate-only regression whose advisory outcome demonstrates the accepted loss, and hard failures for coupling, God path substitution, cycles/layers, identity mismatch, safety, behavior and installation.

This record is active governance; implementation and integration acceptance remain separately tracked in #427/#415. New combined-head CI, same-head delivery artifacts and real installed-entry smoke are required before #420/#421 can complete. No local Cargo execution is permitted while DR-0013 applies.
