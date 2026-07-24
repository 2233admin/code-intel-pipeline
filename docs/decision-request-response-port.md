# Decision request/response port

C06 is the replaceable asynchronous boundary between one C05 Decision Gap and a human-authorized response. It owns correlation and validation only. It does not render UI, choose a transport, grant authority, persist a Decision Record, or couple Pipeline semantics to chat, tmux, or a host application.

## Closed request and response

`code-intel-decision-request.v1` binds exactly one `question` to a gap and correlation id. It carries the proposed recommendation, evidence identities and freshness, options with consequences, the required authority and permitted actors, issue/expiry times, and the exact affected DAG branches. Additional fields fail closed, so a second hidden question cannot be smuggled into the request.

`code-intel-decision-response.v1` binds the same correlation and gap to either a declared option or a non-empty free-form answer. Actor id, authority kind, provenance source, and timestamp are mandatory. A response is rejected before terminal consumption when its correlation, gap, actor, authority, lifetime, processing time, evidence freshness, option, or shape is invalid. Response timestamps later than the processing clock are future-forged and rejected. Once processing time reaches request expiry, no queued response or cancellation can be accepted.

Cancellation is a terminal non-authorizing event (`code-intel-decision-cancellation.v1`): it carries the same correlation, gap, actor provenance, timestamp, and a reason, but produces no engineering or authority effect.

## Replaceable adapters

All adapters implement the same `DecisionRequestResponsePort` interface:

- `InMemoryDecisionPort` is the deterministic test and embedded-process adapter.
- `FileDecisionPort` exchanges bounded JSON files and is the broker-free asynchronous baseline.
- `PlainTextDecisionPort` renders the complete canonical request JSON as transport-neutral text, including recommendation, options/consequences, evidence refs, required authority, and expiry. It accepts an eight-field tab-separated choice, free-form, or cancel reply. It is suitable for outside-tmux and simple CLI hosts without embedding their UI semantics.
- `NativeStructuredDecisionPort` exchanges JSON values for native structured hosts.

The production smoke route uses the native structured adapter:

```text
code-intel decision request-response --request <request.json|-> [--response <response.json>|--cancel <cancellation.json>] --now <unix-seconds> --branch <branch-id>...
```

Adapters can be swapped without changing the request, response, gap, or later C07 Decision Record schemas. No broker or external dependency is required.

## Fail-closed and branch-local behavior

Pending, timeout, and cancellation block only `affectedBranches`; every other supplied DAG branch is returned as `continues`. A valid response makes only the affected branch `ready`. Timeout, cancellation, and a valid response make the correlation terminal, so later replay is rejected. Wrong-gap, wrong-actor, stale-evidence, or malformed responses do not consume the pending request, allowing a later valid response to be correlated safely.

Every exchange result has an empty `effects` list. C06 transports and validates an answer but does not adopt the recommendation, emit an authority event, or commit a plan; those responsibilities remain with A05 and C07.

## Envelope and exit policy

Every invocation writes exactly one `code-intel-decision-exchange-result.v1` JSON envelope to stdout. JSON files and stdin are size-bounded UTF-8 and reject duplicate keys before deserialization.

| Status | Exit | Meaning |
| --- | ---: | --- |
| `resolved` | 0 | One valid correlated response is available. |
| `pending` | 10 | No response is available; only affected branches remain blocked. |
| `timeout` | 11 | Expiry was reached without a response; only affected branches remain blocked. |
| `cancelled` | 12 | A valid non-authorizing cancellation was received. |
| `rejected` | 65 | Usage, JSON, correlation, authority, freshness, clock, or contract validation failed. `diagnostics` is non-empty and no branches or effects are emitted. |

Serialization/runtime failure uses exit 70. Only `resolved` is a successful decision response; pending, timeout, cancellation, and rejection are machine-distinct and never silently reported as success.
