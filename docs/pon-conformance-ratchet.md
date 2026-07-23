# Pon-inspired conformance ratchet

This note selectively reimplements method-level ideas observed in
[`can1357/pon`](https://github.com/can1357/pon) at revision
`ab9067dbd2899c64c4d67a4bc27b8ad49472b126`. The upstream repository did not
declare a license when checked on 2026-07-15, so no implementation code, fixtures, or prose are
copied into Code Intel Pipeline.

## Absorbed semantics

1. **Oracle and floor are separate.** `test-parity-baseline.ps1` remains the byte-stable parity
   oracle for each case. `test-parity-floor.ps1` only records which cases have already earned a
   pass and the minimum passing count.
2. **The proven set is monotonic.** A normal run fails if a committed floor case disappears or no
   longer passes. Updating the floor cannot remove an old case or lower the count.
3. **Progress is explicit.** Passing candidate cases outside the current floor are reported as
   progress; they do not silently change the floor.
4. **Promotion is reviewed.** `-UpdateFloor` requires a non-empty `-ReviewReason`, all candidate
   cases must pass, and the resulting floor remains a committed review surface.
5. **Current truth is machine-owned.** Human documentation may summarize coverage, but the floor
   file and executable oracle own the compatibility claim.

## Local implementation

- `tests/fixtures/parity/parity-floor.json`: committed set-and-count floor.
- `test-parity-floor.ps1`: monotonic checker and guarded updater.
- `test-parity-baseline.ps1`: existing per-case oracle; unchanged by this internalization.

Run:

```powershell
pwsh -NoProfile -File ./test-parity-floor.ps1
```

Promote newly passing fixtures only after reviewing their semantic coverage:

```powershell
pwsh -NoProfile -File ./test-parity-floor.ps1 -UpdateFloor -ReviewReason "<why the new cases are trustworthy>"
```

## Deliberately not absorbed

- The compiler, runtime, CPython corpus, and Rust implementation: unrelated to Pipeline and not
  licensed for reuse.
- A divergence/exclusion ledger: Pipeline evidence is currently small and fail-closed; adding an
  exception mechanism before a real reviewed need would create a regression-hiding surface.
- Automatic floor updates: a green run is evidence, not authority to rewrite the baseline.
- Performance floors: parity correctness is the first sufficient rung; performance ratchets need
  representative timing methodology and a separate noise policy.

## Exit criteria

Remove the source reference if these semantics become independently specified and tested, or
retire the floor wrapper if it never catches a corpus-level regression beyond the per-case oracle.
The existing parity fixtures remain valid if this wrapper is removed.
