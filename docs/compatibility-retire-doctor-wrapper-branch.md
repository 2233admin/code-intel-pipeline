# E09 direct doctor-wrapper retirement (historical)

E09 owned only the three direct production-doctor route segments in `invoke-code-intel.ps1`: the
`check-code-intel-tools.ps1` binding, the preflight invocation block, and its existence guard. It
never owned the retained fresh-machine bootstrap script, B10's Rust adapter, A09, publication,
indexing, Hospital, Native Code Evidence, or provider branches.

**The branch is gone, and it did not go through this gate.** `status.json` records
`decision = retired_out_of_band`, `retired = true`, `deletionExecuted = true`, and
`governanceBypass = true`. The E00 gate output in `gate-out/` is unchanged and still reads
`blocked`. That disagreement is deliberate: it is the record that the branch was removed by
ordinary development rather than because the gate was satisfied. Do not "fix" it by rewriting the
gate decision.

## What actually happened

| Commit | Date | Effect on the three route markers |
|---|---|---|
| `399dd75` Support portable repo paths | 2026-05-26 | full route present — `[1, 2, 1]` |
| `42de063` feat: complete model-independent code intel pipeline | 2026-07-23 | **added this packet and rewrote the wrapper in the same commit** — `[1, 0, 0]` |
| `804e2f0` feat: make compiled CLI the primary entry | 2026-07-25 | last marker removed — `[0, 0, 0]` |
| `372dd83` Merge pull request #13 | 2026-07-25 | deletion lands on `main` |

Two consequences worth stating plainly:

- **The packet was born stale.** `42de063` introduced E09 *and* dropped the frozen three-marker
  route to `[1, 0, 0]` in the same commit. The old verifier asserted `directInvocationCount -eq 2`,
  so that assertion has never held at any commit that contains the packet.
- **The frozen snapshot was never a commit.** The packet declares
  `head: "working-tree-e09"`, `workingTreePolicy: "explicit_overlay"`, and it means it. All 185
  commits were scanned and none reproduces `snapshotIdentity ff712225…`. The `invoke-code-intel.ps1`
  `baseText` alone matches `f3a4e867` "Integrate provider orchestration surfaces (#7)", but
  `doctor_adapter.rs`, `doctor_envelope.rs` and `dag_run.rs` did not exist at that commit, so the
  six-input set was never simultaneously real. Re-deriving the snapshot from the live tree is
  therefore not merely stale — it is impossible, and always was.

## Why this was not driven to an approved retirement instead

The gate's four blockers are preserved verbatim in `status.unmetGateBlockersAtDeletion`. Satisfying
them after the fact would require restoring the branch, running a 30-day compatibility window that
never started, and obtaining an independent approval that does not exist. Manufacturing any of that
evidence would be worse than recording the bypass. The bypass is recorded.

## Regeneration

`New-DoctorWrapperRetirementPacket.ps1` **cannot regenerate this packet** and is retained only for
reference. It requires exactly one match for each of the three route-marker regexes in the live
wrapper and throws `E09 route marker absent or ambiguous` on the first one, because the live
`legacy/invoke-code-intel.ps1` is a thin facade forwarding to `code-intel.ps1` with zero
occurrences of the string `doctor`. The pre-deletion wrapper survives only as the packet's own
rehearsal artifact, `rollback-rehearsal/invoke-code-intel.ps1`.

## Verification

```powershell
pwsh -NoProfile -File legacy/tools/compatibility/Test-DoctorWrapperRetirementPacket.ps1 -PacketRoot orchestration/retirements/e09-doctor-wrapper
```

The verifier is anchored to history, not to the live tree. It checks that the four artifacts share
the one frozen `snapshotIdentity`; that the deletion diff is one bounded three-hunk deletions-only
branch whose `baseText` is content-bound and replays exactly to its declared result; that the
rehearsal artifact is byte-identical to that `baseText` and **no longer** matches the live wrapper;
that all three route markers are absent from the live wrapper and it does not mention `doctor` at
all; that every commit cited in the out-of-band record resolves in this repository; that the E00
decision still reads `blocked`; and that the retained bootstrap was not deleted along with the
branch. The frozen bootstrap hash is deliberately *not* asserted against the working tree —
`check-code-intel-tools.ps1` is a live script and has drifted since the freeze, which E09 has no
authority over.

`PG-015` mirrors the E00 Gain Ledger projection, now carrying `status: "retired_out_of_band"`.
