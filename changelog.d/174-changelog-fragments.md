type: feat

- **changelog.d 碎片化** (#174)：每 PR 写独立碎片，release 时用 `tools/aggregate_changelog.py` 按类型聚合进 CHANGELOG 版本段并删除碎片；CI 对缺碎片的 crates/orchestration 变更仅 advisory 提示，终结 `[Unreleased]` 单行道连环冲突税。
