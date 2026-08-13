# Snapshot-bound Change Impact

`change impact` derives conservative impact and test candidates from the latest A08-admitted run.
It first revalidates the committed evidence and recomputes the current repository snapshot. By
default a stale snapshot is a contract failure: impact is never inferred by mixing evidence from
one checkout state with changed paths from another. Every result carries a `freshness` object
whose `status` is `current` when the recorded and recomputed snapshot identities match.

`--staleness advisory` opts out of the fail-closed default. Mid-edit the working tree has always
diverged from the committed snapshot, so the strict contract makes the query unusable exactly when
writing. In advisory mode a mismatch no longer fails: the walk still runs over the latest committed
run's verified evidence, with the caller's `--changed` paths overlaid as the query input, and the
output labels the gap instead of hiding it. `freshness.status` becomes `stale-advisory`, top-level
`recordedSnapshotIdentity` and `currentSnapshotIdentity` fields expose both identities, and an
extra limitation names the degradation. Advisory answers may be based on an outdated graph and
must never gate; they exist only to prioritize while editing. `--staleness current` is the
explicit spelling of the default, and when the identities match, advisory output is identical to
default output (`freshness.status` stays `current` in both modes).

The v1 implementation walks the verified Native Code Evidence import list in reverse from explicit
`--changed` paths. Exact relative/module resolution is high confidence; a unique suffix resolution
is medium confidence. Impacted test files become the minimal candidates. When the graph reaches no
test, same-module test co-location is an explicit fallback. Returned commands are advisory strings
only and are not executed.

Command planning follows repository evidence instead of a global language default. Rust integration
tests are narrowed to their owning `Cargo.toml` and exact `cargo test --test` targets; non-standard
targets fall back to the owning package and name that extra work in `limitations`. When the import
graph has no test edge, changed Rust source uses crate-local `--lib`/`--bins` unit tests with a
module-name filter instead of expanding the whole co-located test directory. Python uses
`uv run pytest` when a root `uv.lock` is present, otherwise `python -m pytest`. JavaScript and TypeScript
honour the root `packageManager` declaration or lockfile before falling back to npm.

For Rust, use `cargo check` as the fast compile-feedback lane, the returned focused tests while
editing, and the stable Cargo/LLVM full suite as completion authority. Cranelift is intentionally
not selected automatically: it remains a nightly, platform-dependent local build/run experiment,
and should enter a project workflow only after Cargo timings show code generation is the bottleneck.

This policy adapts the workflow proposed in [Rust Cargo Cranelift tuning](https://canmi.net/development/rust-cargo-cranelift-tuning), bounded by the official [Cargo check](https://doc.rust-lang.org/cargo/commands/cargo-check.html), [Cargo timings](https://doc.rust-lang.org/cargo/reference/timings.html), and [rustc_codegen_cranelift](https://github.com/rust-lang/rustc_codegen_cranelift) documentation.

The output names its limitations: the native parser is heuristic and cannot prove runtime calls,
dynamic imports, generated-code edges, reflection, or build-system dependencies. This makes the
result useful for prioritization without presenting it as a semantic proof.
