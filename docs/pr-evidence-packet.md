# PR Evidence Packet

`code-intel pr evidence` turns explicitly supplied, snapshot-bound claim references into one deterministic packet a reviewer can read immediately. It writes the same JSON to `--out` and stdout, so the decision is not hidden behind an artifact path.

```text
code-intel pr evidence --request pr-evidence-request.json --out pr-evidence-packet.json
```

## Claim contract

The request names one repository, base revision, head revision, and snapshot identity. Every claim repeats that snapshot identity in its evidence reference and supplies `artifactSchema`, `type`, and `sha256`; a mismatched identity is rejected. Claims are classified as `gate`, `advisory`, or `observation`, with `pass`, `fail`, or `unknown` status and `current`, `stale`, or `unavailable` availability. A stale or unavailable claim must be `unknown`.

Claims retain zero or more repository-relative `file:line` locations when the underlying tool can name the exact source surface. The packet never invents a location merely to make a claim look grounded.

The deterministic packet ID binds the sorted claims and subject using SHA-256. Its decision includes the authority class, hard-gate status, concise reasons, and an explicit next action:

- A failed `gate` claim produces `blocked`.
- No gate evidence, an unknown gate, stale/unavailable evidence, or a non-gate failure produces `manual_review`.
- Only current passing claims produce `ready_for_human_merge_review`.

`ready_for_human_merge_review` is deliberately not merge authority. Existing required human approval and configured CI checks remain mandatory, and this command never contacts GitHub, invokes a provider, mutates a repository, or merges a PR.

It does not grant merge authority.

## Deliberate limit

This first slice validates the packet and source identities supplied to it; it does not independently re-read every source artifact, publish an A07 committed artifact, or project a result into CI. The follow-on adapter will verify the referenced artifacts and publish/projection-bind this advisory packet without changing this fail-closed decision vocabulary.
