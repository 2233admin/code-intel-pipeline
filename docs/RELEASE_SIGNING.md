# Release Tag Signing

Binary provenance is already covered end to end: `release.yml` attests every
ZIP with `actions/attest-build-provenance` (keyless, GitHub OIDC-backed) and
publishes a SHA-256 sidecar per asset. What that does **not** cover is the git
tag itself — nothing today proves that `v0.7.0` was actually cut by a
maintainer rather than pushed by anyone with write access. This doc closes
that gap with SSH tag signing, gated into `release.yml` for official (GA)
tags only.

## Why SSH signing, not GPG

Most maintainers already have an SSH key registered with GitHub for pushing
over `git@github.com`. SSH signing reuses that same key — no new keypair, no
GPG toolchain to install, no separate identity to manage. `git` has supported
`gpg.format = ssh` natively since 2.34.

## One-time setup (per maintainer who will cut GA tags)

1. Pick (or create) an SSH key you already use with GitHub — public key at,
   for example, `~/.ssh/id_ed25519.pub`.
2. Tell git to sign tags with it:

   ```bash
   git config --global gpg.format ssh
   git config --global user.signingkey ~/.ssh/id_ed25519.pub
   ```

3. Add the same public key to
   [`.github/allowed_signers`](../.github/allowed_signers) in this repo (one
   line per authorized signer — see the file for the exact format), so
   `git verify-tag` and CI can check against it without needing GitHub API
   access. Open a PR with that addition before your first signed tag.

This is a **prerequisite you have to do yourself** — nobody else can generate
or approve your signing identity for you.

## Cutting a signed GA tag

```bash
git tag -s v0.7.0 -m "v0.7.0"
git push origin v0.7.0
```

`-s` signs with whatever `user.signingkey` resolves to. `release.yml` treats
any tag that does **not** match `-beta.<n>` / `-rc.<n>` as GA and will
`git verify-tag` it against `.github/allowed_signers` before building —
an unsigned or unverifiable GA tag fails the release job closed. Beta/rc
tags skip this check entirely, so pre-release cadence is unaffected.

## Verifying a release yourself

```bash
git fetch --tags
git verify-tag v0.7.0
```

Requires `.github/allowed_signers` (or your own copy of it) configured via:

```bash
git config gpg.ssh.allowedSignersFile .github/allowed_signers
```

A clean `Good "git" signature` line means the tag was created by a key listed
in `allowed_signers` — combine with `gh attestation verify` on the ZIP itself
(see [public-beta.md](public-beta.md#install-and-verify)) to confirm both the
tag and the binary trace back to this repository.
