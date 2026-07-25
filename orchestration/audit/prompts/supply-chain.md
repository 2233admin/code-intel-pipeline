# Supply Chain Department Prompt

Adapted from [Fuck_My_Shit_Mountain](https://github.com/XiNian-dada/Fuck_My_Shit_Mountain) `prompts/supply-chain-audit.md` (MIT). Rewritten for Code Intel Pipeline: manifests and workflows are parsed as structured facts first, the model judges only what the parse cannot decide, and output is the fail-closed `code-intel-audit-report.v1` contract.

This prompt is an operating instruction for an agent running the `supply-chain` audit department against a target repository. Read `docs/audit-report.md` and the rubrics in `orchestration/audit/rubrics/` before producing output.

## Applicability gate

The department applies when the target has dependency manifests, a CI configuration, or a release path. Detect and record:

- Manifests and lockfiles: `Cargo.toml`/`Cargo.lock`, `package.json`/`package-lock.json`, `pyproject.toml`/`uv.lock`/`requirements*.txt`, `go.mod`/`go.sum`, and equivalents.
- CI definitions: `.github/workflows/*.yml`, other pipeline configuration.
- Release/packaging surfaces: publish steps, container definitions, install scripts.

No manifests, no CI, and no release path means `not_assessed` with `applicable: "no"` and a reason naming what was searched.

## Structured facts before judgment

Read the files and extract facts; do not infer from names.

- **Manifest ↔ lockfile agreement**: does every declared dependency resolve in the lockfile, and does the lockfile exist at all for an application or service?
- **Pinning**: dependencies on mutable git branches, wildcard ranges, or unauthenticated URLs; toolchain versions pinned or floating.
- **CI trigger and permission surface**: `permissions:` blocks (default vs least-privilege), `pull_request_target` and similar triggers that run fork code with repository secrets, third-party actions referenced by tag or branch rather than a commit SHA, secrets exposed to jobs that execute untrusted code.
- **Execution during install/build**: install hooks, build scripts, `curl … | sh` patterns, native binaries fetched without checksum verification.
- **Release provenance**: artifact checksums or signatures where users download them, container base images pinned by digest, whether the shipped artifact is produced by the same code path that is tested.
- **Package hygiene**: files that would ship in a published package but should not (fixtures, local paths, credentials), license notices.

## Method

1. Parse first: the facts above come from the files themselves, and quoting them makes each finding verifiable.
2. Apply the target's actual risk profile. A private internal tool and a public package with downstream consumers do not get the same severity for the same unpinned action; say which case you are in.
3. Every finding names the compromise path: who has to do what, at which step, to influence the built or published artifact.
4. Separate `confirmed` (the file says so) from `suspected` (behavior inferred but not proven); be conservative per `rubrics/confidence.md`.
5. Score per `rubrics/scoring.md`; report coverage honestly per `rubrics/coverage.md`, including manifests you did not read.

## Boundary with other departments

Code-level injection and secret-handling defects belong to `security`; note the handoff in `exclusions` rather than reporting them here. Model-provider cost and abuse belong to `ai-safety`.

## Secrets red line

A credential found in a manifest, workflow, lockfile, or published package is reported with `redacted: true`, by path and variable name only, with rotation advice.

## Focus

Do not spend output on:

- Advisory-database CVE counts you cannot verify from the tree.
- SBOM or signing requirements for a repository with no public release path.
- Version-bump churn that carries no compromise path.

## Output contract

Produce one `code-intel-audit-report.v1` JSON document. `departments` must list **every** registered department (kernel rule: exact registry membership); report departments whose registry entry is `enabled: false` as `disabled`. Finding ids are `supply-chain-001`, `supply-chain-002`, … Every listed department needs a coverage row; an `assessed` department also needs a non-null score and non-`not_assessed` coverage.

Validate fail-closed and fix until green:

```bash
code-intel audit --operation validate --repo <target-root> --report <report-path>
```
