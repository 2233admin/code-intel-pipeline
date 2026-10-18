# CodeGraph: selective internalization into CodeNexus

Tracking: [issue #429](https://github.com/2233admin/code-intel-pipeline/issues/429). Decision: reuse the replaceable upstream engine; own the Rust adapter, snapshot/cache boundaries, evidence admission and agent-facing commands. The owner approved this choice and isolated branch/CI deployment, not a main merge or release.

CodeNexus is the code/Git perception boundary; `code-intel` owns evidence, artifacts and gates. This implements [ADR 0010](../adr/0010-tool-neutral-engineering-intelligence-core.md), rather than introducing another product called “codegit”. The optional provider is registered but is not enabled in the default DAG. Its internalization lifecycle remains `research` until production promotion is explicitly authorized and attested.

## Compiled entry points

Use a compiled `code-intel` containing this adapter. The artifact directory must already exist **outside** the source repository.

```text
code-intel provider codegraph index --repo . --artifact-root <outside-repo-artifacts>
code-intel provider codegraph status --repo . --artifact-root <outside-repo-artifacts>
code-intel provider codegraph query --repo . --query codenexus_admission --limit 10 --artifact-root <outside-repo-artifacts>
code-intel provider codegraph callers --repo . --query build_active_context --file crates/code-intel-cli/src/codenexus_lite.rs --artifact-root <outside-repo-artifacts>
code-intel provider codegraph impact --repo . --query build_active_context --depth 2 --artifact-root <outside-repo-artifacts>
code-intel provider codegraph affected --repo . --changed crates/code-intel-cli/src/codenexus_lite.rs --artifact-root <outside-repo-artifacts>
code-intel provider codegraph explore --repo . --query "codenexus_admission build_active_context" --limit 3 --artifact-root <outside-repo-artifacts>
code-intel provider codegraph sync --repo . --artifact-root <outside-repo-artifacts>
```

All eleven operations are available: `index`, `sync`, `status`, `query`, `explore`, `node`, `callers`, `callees`, `impact`, `affected`, `files`. `--changed` may repeat for different normalized repository-relative paths. `--file` narrows a symbol query; for `node` it explicitly selects file mode. `--depth` is supported only by operations whose upstream JSON output honors it; `files --depth` is rejected rather than silently ignored. Index/sync are explicit source-read-only cache writes; read commands never initialize or repair an index.

For agent sessions:

```text
code-intel serve --mcp --repo code-intel-pipeline --repo-path . --artifact-root <outside-repo-artifacts>
```

Call `get_code_context` with `{"query":"codenexus_admission build_active_context"}`. The repository and evidence directory are fixed by server startup; tool arguments cannot redirect them or request another operation. Missing engines, absent bindings and stale indexes return errors, not an invented empty complete graph. Query tools do not start a watcher, daemon, downloader or update check.

## What is owned, what remains upstream

| Boundary | Owner |
| --- | --- |
| Parsing, semantic nodes/edges, SQLite engine, upstream CLI/SDK | CodeGraph; replaceable external package |
| Source snapshot, regular-source inventory, repository identity | Existing native `code-intel` snapshot machinery |
| Version/launcher transport, cache binding, lock and stale-read refusal | Rust `codegraph_provider` adapter |
| Persisted payload, A04 validation, MCP contract | Existing `code-intel` admission machinery and adapter |
| Native CodeNexus evidence and authoritative gate decisions | Existing independent paths; unchanged |

The new result contract is `code-intel-codegraph-result.v1`. Read results retain upstream provenance, confidence, ambiguity, truncation, raw stdout/stderr and actual source content. Evidence admission is **advisory / partial / unknown**, with `engineeringFacts: []`. Admission verifies the artifact; it does not promote partial graph observations to authoritative facts or impersonate `codenexus.full`.

## Installation and runtime identity

Reviewed upstream: [v1.6.2](https://github.com/colbymchenry/codegraph/releases/tag/v1.6.2), commit `6560052a6f856855d3f71eee838fd66ccfa4285d`. The floor is stable 1.6.2; newer versions are not forcibly downgraded. CI pins the reviewed version and runs actual upstream contracts on Windows, Linux and macOS.

```text
npm install --global @colbymchenry/codegraph@1.6.2 --ignore-scripts
codegraph --version
codegraph telemetry off
```

On this Windows host, installation exited 0 (`added 2 packages`), version reported `1.6.2`, and a separate telemetry-status invocation confirmed the persisted disabled choice. The adapter additionally sets `DO_NOT_TRACK=1`, `CODEGRAPH_TELEMETRY=0`, `CODEGRAPH_NO_UPDATE_CHECK=1`, `CODEGRAPH_NO_DOWNLOAD=1`, `CODEGRAPH_NO_DAEMON=1` and `CODEGRAPH_NO_WATCH=1` for its children. A saved telemetry choice alone does not prove update checks are disabled for standalone upstream commands.

The npm Windows bundle can be nested under the main package. The adapter resolves the trusted launcher, then invokes its bundled Node and CLI/SDK directly; it does not invoke npm's PowerShell shim or interpolate a shell command. An explicit `CODE_INTEL_CODEGRAPH_BIN` override must be absolute. Provider fingerprints cover the launcher, runtime, SDK entry, CLI entry and package identity—not every transitive dependency byte.

Upstream license: [MIT, copyright 2026 Colby Mchenry](https://github.com/colbymchenry/codegraph/blob/6560052a6f856855d3f71eee838fd66ccfa4285d/LICENSE). No parser or storage implementation was copied into this repository. Preserve the upstream notice when redistributing its package or substantial source. The installed external engine remains a separate dependency.

## Snapshot and cache safety

- `.codegraph/codegraph.db` remains upstream-owned. A separate ignored binding records the source snapshot, exact repository, provider identity and database fingerprint. Queries never search an ancestor repository for a usable cache.
- Source changes invalidate reads until explicit `sync`. Unbound, malformed or externally changed database/provider identities require explicit `index`; `status` diagnoses the mismatch.
- Upstream indexed-file inventory must fit the snapshot's hashed regular-source inventory. Ignored sources, Git links, symlinked sources, escaping paths and ignored/symlinked configuration cannot be silently admitted. Forced ignored inclusions are rejected, preserving the user's configuration.
- Source hashing excludes the ignored database and evidence artifacts. Evidence roots inside the repository, including through parent components, are rejected. Content-addressed payloads referenced by admission remain persisted outside the source tree.
- A repository-local writer lock prevents concurrent adapter cache mutation. A leftover crash lock requires explicit owner inspection/removal; there is no automatic stale-lock takeover.
- Database and nonempty WAL bytes are fingerprinted. An absent WAL and a zero-byte WAL are equivalent: the official read-only SDK creates an empty WAL without changing database contents. Nonempty WAL changes still invalidate the binding.
- Initial indexing uses the official SDK under the lock, not `codegraph init --yes`. In 1.6.2, that CLI initialization with no-watch mode installs Git sync hooks. The adapter does not install hooks or change `.mcp.json`/root ignore files at runtime.

## Observed verification

The original standalone installation indexed 338 files / 7,588 nodes / 22,231 edges. The isolated adapter checkout subsequently indexed 350 files / 7,743 nodes / 22,789 edges and returned a current binding with exit 0. These are observations of different trees, **not** a speed or accuracy comparison. Unsupported sources remain unsupported; this checkout's `.ps1` sources are not semantically indexed. No A/B performance or retrieval-recall improvement has been measured.

[CI run 37843240232](https://github.com/2233admin/code-intel-pipeline/actions/runs/37843240232), at `f1cc45ae3882b7a23b844f8f02cb12420455ef00`, passed on all three platforms. Commands included:

```text
cargo fmt -p code-intel -- --check
cargo test -p code-intel --locked --test codegraph_provider
cargo test -p code-intel --locked --test codegraph_provider -- --ignored --nocapture
cargo test -p code-intel --locked --bin code-intel mcp_serve
cargo build -p code-intel --locked
code-intel lint hardcoded-paths
```

Four public negative tests and four real-upstream conformance tests cover unavailable providers, paths/artifact containment, actual Rust/TypeScript edges/source, duplicate definitions/truncation, index/sync add-change-delete transitions, stale-read refusal, option-shaped query strings and forced ignored-source rejection. Later branch evidence and existing admission/native-CodeNexus contract checks are tracked in issue #429.

The downloaded CI Windows executable was smoke-run locally without Cargo compilation:

- `query`, `callers`, `affected`, `impact` and `explore` each exited 0 against the real checkout. Results contained the requested functions, actual callers, candidate tests and actual source. Five observed envelopes validated against the result/snapshot/admission schemas.
- Actual stdio MCP initialize/tools-list/tools-call returned `get_code_context`; its call succeeded with admitted partial/unknown evidence and no engineering facts. A subsequent `get_gate_verdict` still reported no committed authoritative run. The graph did not manufacture a green gate.
- `code-intel codenexus generate` with an explicitly missing CodeGraph executable exited 0 and produced `codenexus-lite` context (3 files / 36 references), proving the native path does not depend on this engine.
- The repository-root `code-intel sentrux gate .` detected the new route pushing its central catalog beyond 800 lines. Moving that constant into the existing `provider_routes.rs` restored exit 0: `No degradation detected`. The baseline was not reset. A provider-subdirectory gate lacked a scope-specific baseline; repository scope is the calibrated comparison.
- `code-intel change impact --staleness advisory` with the required evidence root and repository arguments returned 65 because no committed authoritative run exists. This is unavailable advisory evidence, not a fabricated impact plan.

This host remains compilation-isolated under [DR-0013](../decisions/DR-0013-affected-host-compilation-isolation.md). All Cargo commands above ran in CI, never on this host. The diagnostic executable is isolated under `$env:LOCALAPPDATA/code-intel/codegraph-429/<revision>/` with its verified adjacent ripgrep runtime. It does not replace the stable `code-intel` entry, update global MCP configuration or activate the current agent session automatically.

## Update, rollback and exit

The machine-readable [internalization record](../../orchestration/internalization/codegraph.json) pins the owned entry point and real conformance test bytes. Re-run the reviewed-version matrix before changing the SDK transport, source/cache rules or supported version floor. Re-review upstream maintenance, licensing and security changes before promoting a newer engine. No source-performance or completeness claim is implied by the compatibility floor.

Rollback is to stop explicit CodeGraph queries / omit `get_code_context`; the native CodeNexus baseline and committed-run gates already work without it. Do not erase referenced evidence payloads as a rollback step. No main merge, release, source-schema migration or default gate takeover is authorized by the branch deployment. A future replacement must preserve snapshot binding, raw provenance/uncertainty, A04 partial semantics, source containment, cross-platform real-engine conformance and the independent native fallback.
