# changelog.d — 碎片化 changelog

每条 PR 的 changelog 条目写在**本目录的独立文件**里，不要直接改根目录 `CHANGELOG.md` 的 `[Unreleased]` 段。

动机见 issue #174：多 PR 并行走 `[Unreleased]` 会连环冲突、反复 rebase，纯税。

## 写碎片

1. 文件名：`<pr号>.md`、`<pr号>-<slug>.md` 或 `<slug>.md`  
   例：`174.md`、`174-changelog-fragments.md`、`span-apply.md`
2. 内容：第一行（或 frontmatter）声明类型，后面是 Keep a Changelog 风格的一条或多条 bullet。

```markdown
type: feat

- **简短标题** (#174)：一两句说明用户/操作者可见的变化。
```

也支持 YAML frontmatter：

```markdown
---
type: fix
---

- **退出码分层** (#130)：gate findings 不再伪装成 process failure。
```

### 类型 → CHANGELOG 小节

| `type` 取值 | 写入小节 |
| --- | --- |
| `feat`, `feature`, `added`, `add` | `### Added` |
| `fix`, `fixed`, `bug`, `bugfix` | `### Fixed` |
| `change`, `changed`, `refactor`, `docs`, `doc`, `documentation` | `### Changed` |
| `remove`, `removed`, `deprecate`, `deprecated` | `### Removed` |
| `security`, `sec` | `### Security` |
| `note`, `notes`, `misc`, `chore` | `### Notes` |

类型大小写不敏感。未知类型会在聚合时报错。

### 不要写进碎片

- README 本身、点文件（`.gitkeep` 等）
- 仅内部重构且操作者不可见的细节（可写 `type: notes` 或省略碎片；CI 对无碎片只 **advisory** 提示）

## 谁何时聚合

**普通 PR**：只添加/修改 `changelog.d/*` 碎片，**不要**手改 `CHANGELOG.md` 的版本段。

**release PR / 打 tag 前**：

```bash
# 预览（不写盘、不删碎片）
python tools/aggregate_changelog.py --version 0.7.0-beta.6 --dry-run

# 写入 CHANGELOG.md 对应版本段，并删除已聚合碎片
python tools/aggregate_changelog.py --version 0.7.0-beta.6
```

默认目标是 `[Unreleased]`（无 `--version` 时）；发版务必带 `--version`。

自检：

```bash
python tests/test_aggregate_changelog.py -v
```

## CI

PR 若改动了 `crates/**` 或 `orchestration/**` 却没有任何 `changelog.d/` 碎片变更，CI 会打出 **advisory** 提示（`::warning::`），**不阻断**合入。docs-only / 纯 chore 可无碎片。
