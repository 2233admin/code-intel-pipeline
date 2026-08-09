# Advisory workflow recommendation

`advisory.workflow-recommend.v2` is the Rust-native, deterministic recommendation capability. It accepts closed semantic intents and required capabilities, reads repository-local configuration evidence, and returns a proposal-only `code-intel-advisory-workflow-recommendation.v2`. The existing `advisory.workflow-recommend` capability is a deterministic v1 projection from the same evaluator.

Both capabilities declare `effects: []`. Recommendation does not initialize a tool, edit the repository, execute a generated action, grant adoption authority, open a pull request, or report an intervention outcome. A configured root such as `openspec/` or `.specify/` proves configuration only. Adoption is reported only when an approved A03-verified authority event is supplied as an Artifact Ref. Competing active normative roots produce a conflict instead of an inferred winner.

## Candidate data and selection

`orchestration/workflow-adapters.v1.json` is the governed candidate catalog. It pins:

- OpenSpec OPSX 1.8.0 at `d57889664cab4f2f061d236ec3ff82a5578701bb`, MIT.
- spec-kit 0.16.1 at `ad4104b56c219b0a27bac06547d1a3c7d6a0dbd6`, MIT.
- the pipeline-owned lightweight adapter for bounded local work.

Selection is capability-driven. OpenSpec covers delta governance, continuous change, and brownfield change. spec-kit covers constitution, clarification, checklists, convergence, composed workflows, and brownfield change. Repository age is not selection authority. A caller may request a manual override only by supplying both the preferred adapter and a non-empty reason; the proposal records that evidence.

Entry, setup, and maintenance actions are separate structured objects. A generated entry or maintenance action is callable only when its generated action is observed for the active host/profile. Setup actions are callable only when their declared prerequisites are satisfied. For example, OpenSpec `explore`, `propose`, `apply-change`, `archive`, and `sync` entries are profile-dependent. `openspec init`, `openspec update`, and `specify init` stay outside normative entry actions and require separate operator authority.

## Host activation wording

Hosts normalize user wording before constructing the closed request. The Rust evaluator does not parse natural language. Typical mappings are:

| User intent | Semantic intent |
| --- | --- |
| “定案”“写方案”“拆规格” | `plan` |
| “开始做”“明确 apply 请求”“按 change 实现” | `implement` |
| “验证”“做完了吗” | `verify` |
| “归档 change” | `archive` |
| “同步规范” | `synchronize` |
| “开 PR”“提交评审” | `ship` |
| “复盘”“效果怎么样” | `observe` |

`ship` and `observe` currently return explicit unavailable handoffs. They belong to the separate shipping-control-loop and outcome-ledger capabilities; recommendation cannot impersonate either one.

## Compatibility surface

The production path is `code-intel capability exec advisory.workflow-recommend.v2`. A01 validates the request and zero-effect envelope; A03 validates recommendation and approved-adoption Artifact Refs; A06/A07 can stage and commit the proposal without changing the default DAG or Hospital authority.

`legacy/Invoke-WorkflowRecommendation.ps1` is retained only as a compiled-CLI forwarder for v1 callers. `legacy/run-code-intel.ps1` also invokes the A01 capability. Historical `-SkipOpenSpec`, `-AutoOpenSpec`, and facade `-Auto` options remain compatibility inputs; none grants adoption or execution authority. The duplicated PowerShell detector has been removed. Rollback is the v1 Rust projection behind the same capability contract, not a second policy evaluator.
