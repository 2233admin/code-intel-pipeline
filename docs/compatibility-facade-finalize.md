# E06 compatibility facade final audit

E06 is a verifier-owned audit surface, not a deletion ticket. It freezes E02, E03, E04, E05,
E07, E08, E09, and E10 as independent prerequisites and refuses to approve the facade while any
packet is missing, blocked, unsigned, or lacks current parity, compatibility-window, or rollback
evidence.

Run the current audit with:

```powershell
pwsh -NoProfile -File Invoke-CompatibilityFacadeFinalize.ps1 `
  -EvaluatedAt ([DateTimeOffset]::UtcNow.ToUnixTimeSeconds()) `
  -OutFile work/e06/final-manifest.json `
  -Json
```

Exit `2` means the audit completed and the result is intentionally blocked. Tool/schema failures
throw and exit nonzero without publishing an approval. The checked-in mode fixtures are explicit
`available=false` records; a clean-machine verifier can provide a separate fixture directory with
doctor/lite/normal/full node observations. Each required node must be registry-backed, enveloped,
and admitted or explicitly not applicable. Non-doctor public modes must also prove A07 commit and
A08 index nodes.

The final manifest uses
`orchestration/schemas/code-intel-compatibility-facade-finalize.v1.schema.json`. Its
`independentApproval` remains `null` in this implementation. Only a verifier who did not author
E02-E05 or E07-E10 may create a later approval artifact, and only after this audit reports no
unsupported branch. The audit never deletes or reroutes PowerShell.

Current retained PowerShell is enumerated in
`orchestration/facade-finalize-policy.v1.json`; it is not assumed to be zero. Missing registry
backing, owner, or future expiry is reported as a named unsupported surface.
