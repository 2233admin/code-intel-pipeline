# Authority transition gate v1

`authority.transition-gate` is a deterministic, provider-neutral policy boundary. Its Rust API is `authority::evaluate_batch(&serde_json::Value)`. The module is intentionally independent of CLI and registry wiring so the runtime coordinator can connect it after shared-file integration is serialized.

The gate distinguishes Observed Evidence, Engineering Fact, Derived Engineering Model, Proposal, Adoption Decision, and Committed Engineering Plan. Evidence-to-fact, fact-to-model, and model-to-proposal transitions are deterministic policy operations. Adoption and commitment transitions require a non-expired, non-replayed `code-intel-authority-event.v1` with a named approver and known supporting evidence. A proposal may be committed directly only when that explicit event owns the transition.

LLMs, providers, and recommenders have no fact or commitment authority. They may originate observations or proposals; when one of their proposals is accepted into an adoption or commitment state, the result records `effectiveAuthority: authority_event`, not the originating tool. The gate does not rank work or choose product priorities.

Requests are evaluated branch by branch. A rejected transition receives no output identifier and does not erase accepted, unrelated analysis. Event replay, duplicate event use, missing approvers, unknown evidence, future or expired events, duplicate branch/output identities, and undeclared edges fail closed.

Accepted protected branches preserve the complete authority event in the result and append its identifier to `consumedAuthorityEventIds`. Persisting that returned set is the caller's replay-state responsibility; the pure gate never silently keeps process-local authority state.

The v1 event remains backward compatible and may additionally carry a
`repository-governed-sha256-v1` attestation. When present, A05 recomputes a canonical SHA-256 over
the event schema, id, decision, approver, sorted evidence ids, issue time, and expiry, and accepts
only an id/role pair in the checked-in `trustedApprovers` policy. This is content-bound repository
sign-off, not cryptographic identity authentication: the digest detects edits, while repository
policy supplies trust. Actor, evidence, expiry, and digest changes therefore fail closed.

The checked policy is `orchestration/authority-transition-policy.v1.json`; the request/event/result contract is `orchestration/schemas/code-intel-authority-transition.v1.schema.json`.
