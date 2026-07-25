# File Boundary Observation

The file-boundary adapter gives the Pipeline a small, provider-neutral way to resolve local rules for one file. It borrows the useful file-addressability idea from AIGX, but it **does not require AIGX**, import its parser, or make `.aigx/` mandatory.

## Contract

The caller supplies a closed `code-intel-file-boundary-request.v1` containing:

- an expected repository snapshot identity;
- one exact repository-relative path;
- a freshness policy;
- a normalized local boundary document with source digest and observation time.

Each document entry can contain a role, `forbid` rules, `gotcha` rules, and checks. Rules and checks use stable IDs so other Pipeline observations can cite them. The adapter performs no command execution and does not modify the source repository.

## Resolution and trust

Resolution is deliberately limited to an exact repository-relative path. Separators and leading `./` are normalized. Absolute paths, parent traversal, wildcards, duplicate normalized paths, case-only ambiguities, duplicate rule IDs, stale observations, future observations, and snapshot mismatches fail closed.

An unmatched file is not treated as unconstrained. It returns `status=unknown`, a null boundary, and `no_matching_boundary`. Unsupported source constructs remain explicit `unsupported_construct` diagnostics; a matched boundary is then `partial`, never silently complete.

The result carries the expected and consumed snapshot identities plus source path, source SHA-256, and observation time. This is Pipeline-owned evidence. A future optional AIGX importer may translate `aigx resolve` output into the local document, but that importer must not bypass this validation boundary.

## Production entrypoint

`code-intel provider file-boundary --request <request.json> --out <result.json>` reads the closed request, resolves the boundary, and writes the closed result. The integration is optional and read-only; AIGX is not a runtime dependency.
