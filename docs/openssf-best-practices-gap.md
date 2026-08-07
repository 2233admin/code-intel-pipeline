# OpenSSF Best Practices (Passing Badge) — Gap List

Per issue #158: the goal here is the gap list, not the badge. This is a
self-assessment against the [OpenSSF Best Practices passing-badge
criteria](https://www.bestpractices.dev/en/criteria/0) (fetched 2026-08-07),
status per criterion: **Met** / **Not met** / **N/A** / **Not verified this
pass** (evidence-gathering limit was reached, not a claim either way).
Re-run this assessment, don't just re-read it, before relying on it for a
release decision more than a few months out — criteria and the repo both
move.

## Basics

| Criterion | Status | Notes |
|---|---|---|
| `description_good` — site explains what the problem is | Met | README.md/README.en.md opening section. |
| `interact` — how to obtain/give feedback/contribute | Not met | Obtain: yes (README install section). Feedback: yes (GitHub Issues). Contribute: no documented process. |
| `contribution` — contribution process documented | Not met | No CONTRIBUTING.md; no PR-process section in README. |
| `contribution_requirements` — acceptance requirements (coding standard) | Not met | Same gap as above; `CLAUDE.md` documents conventions for AI agents working in the repo, not a human contributor-facing standard. |
| `floss_license` — released as FLOSS | Met | MIT, `LICENSE`. |
| `floss_license_osi` — OSI-approved license | Met | MIT is OSI-approved. |
| `license_location` — license posted in standard location | Met | `LICENSE` at repo root. |
| `documentation_basics` — basic docs | Met | README.md/README.en.md, `docs/public-beta.md`, extensive `docs/`. |
| `documentation_interface` — external interface reference docs | Met | CLI help output, `docs/audit-report.md`, `docs/artifact-data-contract.md`, and similar per-surface docs. |
| `sites_https` — website/repo/download over HTTPS+TLS | Met | GitHub-hosted; no non-HTTPS surface. |
| `discussion` — searchable, URL-addressable discussion mechanism | Met | GitHub Issues. |
| `english` — docs/bug reports in English | Not met (partial) | `README.en.md` exists, but most in-repo documentation (CHANGELOG, code comments, most of `docs/`) is Chinese-first; no stated policy that English bug reports/comments are accepted. |
| `maintained` — project is maintained | Met | Active commit history through the date of this assessment. |

## Change Control

| Criterion | Status | Notes |
|---|---|---|
| `repo_public` — public version-controlled repo with URL | Met | `https://github.com/2233admin/code-intel-pipeline`. |
| `repo_track` — tracks what/who/when | Met | Git. |
| `repo_interim` — interim versions between releases, not just finals | Met | Every commit lands on `main`, not just tagged releases. |
| `repo_distributed` (SUGGESTED) — DVCS | Met | Git. |
| `version_unique` — unique version id per release | Met | SemVer-shaped tags (`vX.Y.Z[-beta.N]`), cross-checked against `Cargo.toml` by `release.yml`. |
| `version_semver` (SUGGESTED) | Met | SemVer, including prerelease suffixes. |
| `version_tags` (SUGGESTED) — releases identified by git tags | Met | `v*` tags. |

## Reporting

| Criterion | Status | Notes |
|---|---|---|
| `vulnerability_report_process` — published vulnerability-reporting process | Not met | No `SECURITY.md`; no vulnerability-specific reporting section in README (general GitHub Issues only, which is not a private channel). |
| `vulnerability_report_private` — private reporting path documented | Not met | Same gap; GitHub's private vulnerability reporting feature is not enabled/documented for this repo. |
| `vulnerability_report_response` — ≤14-day initial response, last 6 months | Not verified this pass | No vulnerability reports have been filed through any documented channel to measure against. |
| `report_process` — bug-report process | Met | GitHub Issues, actively used (see the issue history referenced throughout this repo's CHANGELOG). |
| `report_tracker` (SUGGESTED) — issue tracker used | Met | GitHub Issues. |
| `report_responses` — majority of bug reports acknowledged, last 2-12 months | Not verified this pass | Would require pulling issue response-time statistics via `gh api`; not done this pass. |
| `enhancement_responses` (SUGGESTED) | Not verified this pass | Same as above. |
| `report_archive` — public archive of reports/responses | Met | GitHub Issues is itself the searchable archive. |

## Quality

| Criterion | Status | Notes |
|---|---|---|
| `build` — working automated build | Met | `cargo build`, wired into every CI workflow. |
| `build_common_tools` (SUGGESTED) | Met | Standard Cargo/rustup toolchain. |
| `build_floss_tools` — buildable with only FLOSS tools | Met | Rust toolchain, ripgrep, Python — all FLOSS. PowerShell 7+ (`pwsh`) is required by the installer/launchers; PowerShell 7 is FLOSS (MIT) and cross-platform, so this still holds. |
| `test` — automated, publicly released, documented test suite | Met | `cargo test`, Python `tests/*.py`; invocation documented in `release.yml`/`ci.yml` and this repo's own docs. |
| `test_invocation` (SUGGESTED) | Met | Standard `cargo test` / `python -m unittest` invocations. |
| `test_most` (SUGGESTED) — most branches/functionality covered | Not verified this pass | No line/branch coverage tool is wired into CI to measure this against; the audit report's `supply-chain` finding does not cover test quality (out of this department's scope — `security`/`quality` territory). |
| `test_continuous_integration` (SUGGESTED) | Met | `ci.yml` runs on every push to `main`; `pr-gate.yml` on every PR. |
| `test_policy` — policy that new functionality gets tests | Not verified this pass | No written policy found; this repo's practice (extensive tests accompanying feature commits, visible throughout CHANGELOG.md) suggests the practice exists, but it is not documented as a stated policy. |
| `tests_are_added` — evidence the policy is followed | Met (by practice, not by written policy) | CHANGELOG.md entries consistently pair new functionality with new/updated tests (e.g. the `#206` and `#151` entries). |
| `tests_documented_added` (SUGGESTED) | Not met | Not documented anywhere contribution-facing (see `contribution` gap above — there is no contribution-facing doc at all yet). |
| `warnings` — compiler warnings / lint tool enabled | Met (partial) | `cargo build`/`cargo fmt -- --check` run in CI; no `clippy` (or any dedicated lint tool beyond rustc's own warnings) is wired in. |
| `warnings_fixed` — warnings addressed | **Not met** | `cargo build -p code-intel --bin code-intel` on the current tree (run during this audit) reports **177 warnings**, 0 errors — unaddressed dead-code/unused-function warnings, not zero as this criterion requires. |
| `warnings_strict` (SUGGESTED) | Not met | Follows from the above; no `-D warnings` or clippy strict mode. |

## Security

| Criterion | Status | Notes |
|---|---|---|
| `know_secure_design` / `know_common_errors` | Not verified this pass | Requires maintainer self-attestation, not something derivable from the tree. |
| `crypto_published` / `crypto_call` / `crypto_floss` | N/A (mostly) | This project's own code does not implement or call cryptographic primitives directly; it shells out to `git`, `gh`, and standard OS/Rust-ecosystem tooling for the one crypto-adjacent operation it has (release-artifact signing/attestation via GitHub's Sigstore-backed `attest-build-provenance`, which is itself FLOSS and publicly reviewed). |
| `crypto_keylength` / `crypto_weaknesses` | N/A | No custom cryptographic mechanism to configure key lengths or algorithms for. |
| `delivery_mitm` — delivery mechanism resists MITM | Met | GitHub Releases over HTTPS; SHA-256 checksum plus (as of this assessment) GitHub Artifact Attestation verification in the install path — see `docs/release-provenance-runbook.md`. |
| `delivery_unsigned` (MUST) — a hash MUST NOT be fetched over plain HTTP and trusted without a signature check | Met | This criterion is about *not* trusting an HTTP-retrieved hash unsigned, not about signing releases (that's `signed_releases`, corrected below). Every checksum this project publishes or fetches (GitHub Release `.sha256` sidecars, the Skill installer's download) is served over HTTPS end to end — there is no plain-HTTP hash-retrieval path to be unsigned in the first place. |
| `vulnerabilities_fixed_60_days` | Not verified this pass | No publicly known vulnerability has been filed against this repo to measure against; revisit if one ever is. |
| `vulnerabilities_critical_fixed` (SUGGESTED) | Not verified this pass | Same as above. |

**Correction (post-review):** this assessment originally mislabeled `delivery_unsigned` (Passing, MUST) as the criterion covered by this project's GitHub Artifact Attestation work and as `SUGGESTED`. Neither was right. The criteria that actually cover signed release artifacts and signed tags — `signed_releases` (MUST) and `version_tags_signed` (SUGGESTED) — are **Silver-badge criteria**, one tier above what this document assesses (`bestpractices.dev/en/criteria/1`, not `/0`), so they are out of this Passing-badge gap list's scope by definition. Noted here anyway because they're exactly what issue #158 is about:

| Silver criterion (out of Passing scope) | Status | Notes |
|---|---|---|
| `signed_releases` (MUST for Silver) | Arguably met | GitHub Artifact Attestation (Sigstore/OIDC) cryptographically signs every release ZIP with no private key to manage or leak — verified end-to-end against the real, published `v0.7.0-beta.6` release for all three platforms during this audit (see `docs/release-provenance-runbook.md`). The criterion doesn't mandate a specific signing mechanism; keyless Sigstore/OIDC is a recognized modern approach to it. Not claiming a formal "Met" since this document only assesses Passing. |
| `version_tags_signed` (SUGGESTED for Silver) | Not met | Release tags are unsigned — tracked as `supply-chain-001` in `orchestration/audit/reports/audit-report.json` and in `docs/release-provenance-runbook.md`. |

## Analysis

| Criterion | Status | Notes |
|---|---|---|
| `static_analysis` — static analysis tool beyond compiler warnings | **Not met** | No `clippy`, no other static analyzer wired into CI or the release pipeline. This repo's own `code-intel` self-scan (Sentrux structural gate) analyzes architecture/complexity, not general static-analysis-class defects (unsafe patterns, common vulnerability classes) — it does not substitute for this criterion. |
| `static_analysis_common_vulnerabilities` (SUGGESTED) | Not met | Follows from the above — no tool means no vulnerability-pattern ruleset either. |
| `static_analysis_fixed` | N/A (no tool run) | Nothing to have fixed yet; becomes live once `static_analysis` is met. |
| `static_analysis_often` (SUGGESTED) | Not met | Same. |
| `dynamic_analysis` (SUGGESTED) | Not met | No fuzzer or dynamic analysis tool in CI. |
| `dynamic_analysis_unsafe` (SUGGESTED) | N/A | Core is Rust (memory-safe by default); no `unsafe` blocks were flagged during this pass, and the project is not written in a memory-unsafe language, so this SUGGESTED criterion is N/A by its own text. |
| `dynamic_analysis_enable_assertions` (SUGGESTED) | Not verified this pass | `debug_assert!`/`assert!` usage exists throughout the Rust test suite (standard Rust testing idiom); whether this specifically satisfies the criterion's intent was not evaluated. |
| `dynamic_analysis_fixed` | N/A (no tool run) | Same as `static_analysis_fixed`. |

## Summary: what actually blocks the badge today

Real, fixable gaps found this pass, roughly in order of effort:

1. **No `CONTRIBUTING.md` / no documented contribution process** — blocks `interact`, `contribution`, `contribution_requirements`, `tests_documented_added`. Cheapest fix on this list.
2. **No `SECURITY.md` / vulnerability-reporting process** — blocks `vulnerability_report_process`, `vulnerability_report_private`. Also cheap — mostly a doc, optionally enabling GitHub's private vulnerability reporting feature.
3. **177 unaddressed compiler warnings, no lint tool wired into CI** — blocks `warnings_fixed`, `static_analysis`, and their SUGGESTED siblings. The largest real-effort item here; `cargo clippy` wiring is small, clearing the existing warning backlog is not.
4. **Release tags are unsigned** — already tracked as `supply-chain-001` in this audit's findings and in `docs/release-provenance-runbook.md`. This is the Silver-tier `version_tags_signed` gap, not a Passing-badge blocker; noted because it's directly what issue #158 asked about.

Everything marked **Not verified this pass** needs a maintainer with response-time/issue-history access (or a `gh api` sweep) to resolve one way or the other — this assessment did not fabricate a status for those.
