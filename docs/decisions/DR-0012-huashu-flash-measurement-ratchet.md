# DR-0012 huashu-flash measurement ratchet

Status: active
Date: 2026-09-26

## Decision

Add `measurement.flash-ratchet` as its own Capability Atom. It computes
p50/p75/p95 for caller-supplied Measurement Samples and holds a Flash
Ratchet. It does not run the Measured Operation, spawn a process, open a
browser, or authorize publication.

The method is the one in `alchaincyf/huashu-flash` (`scripts/bench.py`,
`scripts/ratchet.py`, MIT, Copyright 2026 花叔 Huashu), narrowed to the part
that is not web-specific:

1. A failed attempt is recorded with its reason and is excluded from the
   percentiles. Ten means ten finite samples.
2. The tolerance is five percent, stored in the Ratchet Ceiling. A check
   reads the submitted record, not a code constant.
3. Metrics only improve by getting smaller. There is no `higher` direction.
4. A Paired Comparison is optional. When present, the caller submits the
   collection order. An order that does not strictly alternate is rejected
   and no reduction is reported. A valid pair reports the fractional p75
   drop from the first group to the second.

`delivery.light-speed-measure` stays the delivery-path atom. Its timing
events are one interval per event, not repeated samples, and it has no
ceiling.

## Why

The user asked for the huashu-flash method as a product feature usable for
code and other operations, not only web pages, and then asked to follow the
upstream project rather than invent local policy. Upstream's own scripts
already decide failure exclusion, the five-percent tolerance, lower-only
metrics, and the p75 reduction. Copying its Playwright runner would make
the atom a browser. Leaving the ceiling as a constant would re-judge an old
record after the constant changed. `docs/pon-conformance-ratchet.md` already
defers performance ratchets for lack of a noise policy; this record is that
policy.

## Enforcement

- `crates/code-intel-cli/src/flash_ratchet.rs` implements the kernel.
- `orchestration/schemas/code-intel-flash-ratchet.v1.schema.json` is the
  artifact contract.
- `crates/code-intel-cli/tests/flash_ratchet.rs` pins init, hold, tighten,
  regress, excluded failures, and rejected non-alternating order.
- convention only until that test exists; this record does not change
  runtime behavior by itself.
