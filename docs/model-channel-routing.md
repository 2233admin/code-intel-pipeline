# Model channel routing

Model execution is optional. Repository snapshotting, inventory, graph extraction, Sentrux checks, diagnosis, and artifact publication remain deterministic when no model channel is ready.

The runtime separates four concerns:

1. A **channel** executes model work (`ollama`, a user-supplied compatible endpoint, Claude Code, OpenCode, or Codex).
2. A **configuration broker** such as CC Switch may indicate configuration presence, but is not itself a model channel and never grants permission.
3. The **routing policy** records explicit consumption, external-data, and paid-spend authorization.
4. The **adapter** invokes only the selected ready channel after policy evaluation.

Inventory is observation only. It must not contain credential values, endpoint URLs or query strings, prompts, model responses, tokens, or credential-store contents. `endpointConfigured` is only a boolean presence signal. `externalEgress` states whether using a channel would send workload data beyond the local trust boundary.

## Readiness and authorization

Candidates progress through this closed state chain:

`discovered -> executable_verified -> auth_present -> model_available -> egress_allowed -> spend_allowed -> ready`

Authentication is evidence of access configuration, not permission to consume it. `authPresent=unknown` cannot become ready; local channels may use `not_applicable`. Consumption authorization uses `unanswered`, `granted`, or `denied` and separately names allowed cost scopes:

- `local_compute`
- `subscription_cli`
- `free_or_internal_quota`
- `metered_api`

Repository-data egress and paid API spend each require their own `granted` decision. A pinned adapter is evaluated first. When it is unavailable, fallback occurs only with `fallbackPolicy=allowed`.

## Outcomes

`code-intel model route` emits one of:

- `ready`: a selected channel plus non-secret execution metadata.
- `consent_required`: a usable channel is blocked by an unanswered or denied authorization. The command exits 2 and performs no model call.
- `deterministic_degraded`: no eligible model channel exists. The command exits 0 so deterministic pipeline stages continue; the LLM-dependent node records `manual_required` and may emit an assistance dossier.

Contract or protocol violations exit 65, usage errors exit 64, and local I/O failures exit 74. Routing attempts contain only candidate IDs, readiness state, a closed failure category, and a stable reason code.
