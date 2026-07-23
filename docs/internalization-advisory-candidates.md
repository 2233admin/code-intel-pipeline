# Advisory candidate internalization records

R01 through R04, R13 through R20, and R22, are canonical research records under
`orchestration/internalization/`. They are data and provenance records, not dependency installs,
workflow execution, or adoption decisions.

The local workspace proves the current candidate semantics, the B06 zero-effect authority boundary,
the pipeline-owned files, and small local measurements. It does **not** contain trustworthy upstream
commit and license evidence for these references. Each record therefore pins the exact local
evidence bytes with SHA-256, labels the upstream revision/license as unverified, includes explicit gap
evidence IDs, and remains in `research`. Validation intentionally omits those gap IDs from admitted
evidence, so the engine emits diagnostics and keeps `productionEnabled=false`.

| Record | Locally measured evidence | Unresolved production blockers |
| --- | --- | --- |
| `internalization.repowise-record` | B01 adapter and conformance SHA-256 plus 2 registered production operations | upstream revision/license, CLI compatibility, quota/security/maintenance, representative value/cost, exit and retirement proof |
| `internalization.graph-record` | B02 adapter and conformance SHA-256 plus separately traced internal and external implementations | external revision/license/runtime/conformance, maintenance/security, representative utility/cost, exit and retirement proof |
| `internalization.sentrux-record` | B03 adapter and conformance SHA-256 plus registered adapter/runtime operations | upstream revision/license, Windows/plugin conformance, maintenance/security, representative value/cost, shim retirement proof |
| `internalization.codenexus-record` | B04 adapter and conformance SHA-256 plus full/lite swap and registered production operations | full-provider revision/license/runtime/security/maintenance, measured localization value/cost, exit and lite retirement proof |
| `internalization.openspec-record` | 5 `openspec-opsx` occurrences in the current advisory atom | upstream revision, license, update, security |
| `internalization.spec-kit-record` | 8 `spec-kit` occurrences in the current advisory atom | upstream revision, license, update, security |
| `internalization.matt-flow-record` | 1 matt-flow candidate branch | upstream revision, license, update, security |
| `internalization.gstack-record` | 1 gstack candidate branch | canonical source, upstream revision, license, update, security |
| `internalization.qiaomu-goal-record` | 7 locally documented goal-contract semantics | upstream revision, license, upstream conformance, update, security |
| `internalization.agent-loops-record` | 3 locally documented loop-pattern choices | upstream revision, license, upstream conformance, update, security |
| `internalization.metaharness-record` | 6 locally documented harness design concerns | upstream revision, license, upstream conformance, retention authority, update, security |
| `internalization.yao-meta-skill-record` | 8 locally documented skill benchmark criteria; local test explicitly is not upstream execution proof | upstream revision, license, upstream conformance, behavioral measurement, update, security |
| `internalization.mattpocock-skills-record` | 4 independently listed absorbed concepts | upstream revision, license, update, security |

Each record contains its own adoption rung and owned boundary, compatibility/conformance evidence,
measured benefit/cost, maintenance/security and update gaps, owned modifications, rollback,
replacement/exit criteria, and retirement triggers. Reuse and NOTICE views are deterministic C03
projections only; they cannot authorize production, initialization, external writes, or commitments.
