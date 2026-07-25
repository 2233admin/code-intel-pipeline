# R05-R12/R21/R23-R26 Internalization Records

Status: implemented as canonical lifecycle records; not independently approved.

These records separate locally recomputable implementation facts from missing upstream or authority evidence. Research is fail-closed; explicit out-of-scope/defer outcomes require a checked-in repository sign-off.

| Ticket | Record | Current result | Hard boundary |
| --- | --- | --- | --- |
| R05 | `orchestration/internalization/repomix.json` | Fresh audit distinguishes one npm registry-metadata cache entry from zero executable/package payloads; B07 reviewed deletion and zero production call sites remain reconciled | No install or production restoration without pinned provenance, conformance, measurements, and new authority |
| R06 | `orchestration/internalization/native-code-evidence.json` | In addition to B08 parity, a 12-sample manually labeled multilingual corpus runs the real native producer: TP=6, FP=2, FN=2, TN=2, precision=0.75, recall=0.75, supported-language coverage=10/12 | Metrics describe line-heuristic symbol detection only; AST, call-graph, framework, and relationship precision remain outside the claim |
| R07 | `orchestration/internalization/cocoindex.json` | Installed 0.2.37 provenance remains audit-only; B07 marks the participant deleted, removes the integration, configuration lookup, and command discovery; semantic invocations remain zero | Native Code Evidence stays independent; the legacy outcome is a static compatibility tombstone, not a declared disabled provider |
| R08 | `orchestration/internalization/github-research.json` | Authenticated representative blocker query took 12228.7508 ms, returned `manual_required`, zero candidates, resolution@k=0, and `invalid-query`; B07 reviewed deletion removes production network/credential calls | Offline controls are retained only as historical adapter evidence and are not counted as blocker-resolution value |
| R09 | `orchestration/internalization/rg.json` | Current required invocation is traced; lifecycle remains `research` until package provenance, retained license, platform matrix and replacement drill close | No implicit `rg` upgrade or replacement |
| R10 | `orchestration/internalization/git.json` | Git 2.54.0.windows.1 and its local GPL-2.0-only license digest are bound; the provider-neutral alternate-VCS fixture tests mismatch exit 65/no artifacts and rollback to Git or unversioned explicit overlay | Snapshot adapter is read-only; an actual alternate implementation must still pass the full platform fixture matrix |
| R11 | `orchestration/internalization/tree-sitter-v.json` | MIT notice, local overlay hashes, compiled Windows artifact digest, and ABI-alias source digest are recorded; pinned upstream revision, reproducible build, exported-symbol and V-fixture evidence remain open | Pipeline owns overlay/ABI glue, not Sentrux parsing or the grammar |
| R12 | `orchestration/internalization/greenfield.json` | External plugin participation is retired because source/license/effects/reviewed generated-spec value were not bound; only the pipeline-owned plan-only handoff remains, with two fixture paths and zero auto-analyze | The plan contract is not plugin completion and grants no specification or implementation authority |
| R21 | `orchestration/internalization/ponytail.json` | C00 source/test hashes and behavioral boundary are recorded; upstream provenance and measured value remain open | Governance concepts are not an external Ponytail runtime |
| R23 | `orchestration/internalization/linear.json` | Expiring repository-signed `out_of_scope` decision | No connector, credential, API call, issue write, or second task authority |
| R24 | `orchestration/internalization/obsidian.json` | Expiring repository-signed `out_of_scope` decision | No UI/plugin/vault dependency; scanner artifacts remain authority |
| R25 | `orchestration/internalization/llm-wiki.json` | Expiring repository-signed `out_of_scope` decision | No model/provider/data effect; generated prose cannot promote itself to fact |
| R26 | `orchestration/internalization/my-code-machine.json` | Expiring repository-signed defer (`out_of_scope`) decision reconciling ADR 0001 with ADR 0010 | No big-bang merge, host mutation, sync, or migration authority |

Each record projects through C03 to Reuse and NOTICE documents. The shared measurements are bound
by SHA-256 from `orchestration/internalization/c03-r05-r12-measurements.json`. Unresolved `gap:*`
evidence keeps R05-R12, R21 in research with `productionEnabled: false`; R05 additionally has a
completed implementation retirement and reviewed B07 production deletion. R23–R26 are explicitly
`out_of_scope`; their content-bound, expiring attestations bind approver, decision, evidence, issue
time, and expiry. They are repository-governed sign-offs, not cryptographic identity
authentication. R09 describes an observed existing production dependency; that observation does
not retroactively approve its lifecycle, and representative latency p50/p95 remains an explicit
measurement gap.
