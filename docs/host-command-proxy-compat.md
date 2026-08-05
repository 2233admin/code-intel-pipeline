# Host Command Proxy Compatibility

Agent hosts increasingly wrap their shell in token-optimizing command proxies
(e.g. RTK-style PreToolUse rewriters). Three behaviors of that layer threaten
verdict integrity when `code-intel` runs under it:

1. **Command rewriting.** The proxy substitutes "equivalent" tools with
   different semantics (`grep` → `rg`, `find` → `fd`); a rewritten judgment
   command may fail or judge something else.
2. **Cache replay.** The proxy may answer a byte-identical command line with a
   cached capture of an earlier run — stdout, stderr, and exit code together.
   A fixed defect stays red, an unfixed one stays green, and nothing in a
   replayed verdict says so.
3. **Output filtering.** The proxy compresses or truncates output; a verdict
   that exists only as stdout text can be dropped.

## Standing defenses

- **Verdicts ride exit codes.** `run execute` reports through exit codes
  (`0`, `10` architecture/domain gate failure, `70` process failure, `73`
  publication collision), as do the other gate commands. Output filtering
  cannot change a verdict.
- **Judgment invocations never repeat a command line.** `run execute`
  requires a fresh `--out` directory and a unique `--final-name`; the
  benchmark requires a fresh `--out`. Two honest invocations therefore never
  share command-line bytes, so a byte-keyed cache cannot serve one for the
  other. This uniqueness is part of the anti-replay contract, not an
  accident of the publication design.
- **Invocation-identity echo.** Verdict-producing commands (`run execute`,
  `run dag-coordinate`, `audit`, `benchmark`) print one line to stderr before
  any other output:

  ```
  invocation-identity: command=run-execute id=<nanos-pid-seq hex> at=<UTC RFC3339>
  ```

  This is deliberately not the manifest `runIdentity`: that identity is
  digest-derived and stays stable for identical content, while this line is
  nondeterministic by design — one names the artifact, the other proves the
  invocation happened.

  The bytes differ on every invocation. A replayed capture exposes itself by
  a repeated `id=` or a stale `at=` clock. The line is stderr-only — no
  stdout JSON contract changes shape — and it never enters manifests,
  reports, or digests, so artifact determinism contracts are unaffected.
  Contract probes (`--contract-probe`) stay silent: they are catalog
  introspection, and the head-parity fixture byte-compares their streams.
  `doctor` also stays silent for now — its envelope contract asserts an empty
  stderr — so its identity has to ride inside the envelope itself (#197).

## Guidance for proxy hosts

- Trust exit codes, not prose, for pass/fail.
- Run judgment commands through a pass-through channel (e.g. `rtk proxy`, or
  a shell surface the proxy does not intercept) when the proxy caches or
  rewrites.
- When the same judgment command must run twice (test reruns, flake checks),
  compare the `invocation-identity` lines: identical lines mean the second
  run never happened.
- Prefer recoverable compression (`repowise distill` / `expand`) over lossy
  filtering for noisy command output; distilled output preserves exit codes
  and every omitted line stays retrievable.
