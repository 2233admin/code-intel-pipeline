# Native Code Evidence capability atom

`evidence.native-code` is the deterministic B08 baseline behind the A01 capability envelope and
the A09 run DAG. It consumes exactly one A03-verified `inventory.files` Artifact Ref plus the A02
Snapshot Identity in the request. Repository reads are guarded by a snapshot consumption lease;
the capability fails closed if the repository changes during extraction.

The atom preserves the stable v1 machine projections for files, heuristic symbols, file-sized
chunks, symbol containment, heuristic imports, the scorecard, and Agent Code Slice ranking under
`code-evidence/merged/`. Every machine projection is returned as a digest- and snapshot-bound
Artifact Ref and is reverified by A03 before A09 exposes it to downstream nodes. Markdown Agent
Code Slice views are deterministic rebuildable views of those machine artifacts.

Coverage is deliberately bounded. The built-in extractor uses line heuristics for PowerShell,
Python, JavaScript/TypeScript, Rust, Go, and Java. Other files remain in files/chunks but appear in
`code-evidence/coverage.json` as unsupported. Symbol and import precision are `heuristic`;
relationship precision and call-graph status are always `unknown`. The atom never emits a
call-graph artifact and makes no claim about external cocoindex or specialized semantic graphs.

Declared and observed effects are exactly `repo_read` and `local_write`. The existing embedded
PowerShell producer remains the compatibility/rollback implementation until later facade
retirement tickets complete parity and independent verification.
