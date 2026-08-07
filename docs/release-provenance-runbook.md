# Release Provenance Runbook

Maintainer-facing process for the release-provenance guarantees issue #158
asked for: this repository must pass the same supply-chain bar its own
`supply-chain` audit department holds other projects to. User-facing install
and verify steps live in [`docs/public-beta.md`](public-beta.md); this
document covers the parts only a maintainer cutting a release does.

## What's already automatic

Every tag push matching `v*` runs `.github/workflows/release.yml`, which
after every platform build, self-scan, and packaging gate passes:

1. Signs all three platform ZIPs with a [GitHub Artifact
   Attestation](https://docs.github.com/actions/security-guides/using-artifact-attestations)
   (`actions/attest-build-provenance`, keyless/OIDC — no private key to
   manage or leak).
2. Publishes the ZIP, its `.sha256` sidecar, and a `.release-manifest.json`
   to the GitHub Release.

Nothing below needs to happen for that part — it already runs on every tag.

## Recorded verification (2026-08-07, v0.7.0 GA prep)

Acceptance criterion from issue #158: *"the three platform ZIPs downloaded
from a GitHub Release all pass `gh attestation verify`, the verification
command is in the README, and it has actually been run once on a clean
machine and recorded."* Run against the already-published `v0.7.0-beta.6`
release (the most recent tag built after the attestation step landed in
`ad67738`, 2026-07-28):

```powershell
gh release download v0.7.0-beta.6 -p 'code-intel-pipeline-v0.7.0-beta.6-windows.zip'
gh attestation verify code-intel-pipeline-v0.7.0-beta.6-windows.zip --repo 2233admin/code-intel-pipeline
# exit 0 -- 1 attestation verified, Sigstore certificate chain resolves to
# https://github.com/2233admin/code-intel-pipeline/.github/workflows/release.yml@refs/tags/v0.7.0-beta.6
```

Repeated for `-macos.zip` and `-linux.zip`: both also exit `0`, one
attestation each, same workflow/tag binding. All three platform ZIPs from a
real, already-published release verify cleanly on a machine that only has
`gh` — no source checkout, no build tooling. This satisfies the acceptance
criterion; it does not need to be re-run for every future release (the
mechanism, not one instance of it, is what's being proven), but should be
spot-checked again if `release.yml`'s attestation step ever changes.

Full findings and coverage from the department run this verification was
part of: [`orchestration/audit/reports/audit-report.json`](../orchestration/audit/reports/audit-report.json)
(`code-intel audit --operation render --repo . --report orchestration/audit/reports/audit-report.json`
to read it formatted).

## What's still manual: signing release tags

`git tag -s` is not yet wired into the release process — issue #158's
second acceptance criterion (`git tag -v` passes for at least one tag) is
not yet satisfied. Until it is, tag authenticity rests entirely on who has
push access to this repository (see the audit finding
`supply-chain-001` in the report above: there is currently no tag-protection
ruleset either).

To cut a signed tag:

```bash
git tag -s v0.7.0 -m "v0.7.0"
git tag -v v0.7.0          # verify locally before pushing
git push origin v0.7.0
```

This requires a GPG (or SSH, with `git config gpg.format ssh` and
`user.signingkey`) key the maintainer controls, configured with
`git config user.signingkey <key-id>` and `git config tag.gpgSign true`.
That key is **not** committed anywhere in this repository or its CI
secrets — signing happens on the maintainer's machine before the tag is
pushed, the same way GitHub's own signed-commit workflow works. Document
the actual key fingerprint used for the first signed tag here once it
exists; do not commit the private key material itself under any
circumstance.

Recommended follow-up (tracked in #158, not yet done): add a repository
ruleset restricting who can push tags matching `v*`, so an attacker with a
narrowly-scoped leaked token cannot cut a release even before the signature
check exists.

## OpenSSF Best Practices self-assessment

Gap list against the [OpenSSF Best Practices
Badge](https://www.bestpractices.dev/) criteria:
[`docs/openssf-best-practices-gap.md`](openssf-best-practices-gap.md).
Per issue #158: the badge itself is not the goal, the gap list is — it is a
free, externally-defined supply-chain checklist that feeds the
`supply-chain` audit department's own future detector registry (#138).
