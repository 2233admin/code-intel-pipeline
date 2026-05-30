# Changelog

## v0.1.1 - 2026-05-30

Release infrastructure patch.

- GitHub Actions now exports the installed Code Intel tool bin directory through `GITHUB_PATH`, so later CI steps can find the Sentrux shim.
- Release workflow is idempotent: if a GitHub Release already exists for a tag, it uploads or replaces the zip asset instead of failing.
- Release package avoids bundling local `pipeline.config.json`; it ships `pipeline.config.example.json` instead.

## v0.1.0 - 2026-05-30

Code Intel Pipeline 的第一个公开版本。

这一版把本地代码理解工具链整理成一条可重复的流程：刚从 GitHub clone 下来的项目，先摊成地图，再交给 Agent 动手。

- 便携安装器、doctor、自检脚本和一条命令入口。
- 串起 `rg`、Repowise、Understand Anything、Sentrux、CodeNexus-lite。
- 大仓库支持 scoped Repowise，避免根目录里的外部轮子污染判断。
- Governance 状态机输出 `hospital-report.json` 和 `surgery-plan.md`。
- Sentrux Agent 工具：`scan`、`health`、`session_start`、`session_end`、`rescan`、`check_rules`、`evolution`、`dsm`、`git_stats`、`test_gaps`、`what_if`。
- Sentrux lite fallback 和开源部署下的本地 auto-Pro 激活。
- Windows 下的 Sentrux V language 插件覆盖包。
- GitHub Actions Windows smoke test。
- 中文 README、GPT娘横幅、实际部署说明。

已知边界：

- Understand Anything 图谱生成仍依赖宿主 Agent skill。如果缺 `.understand-anything/knowledge-graph.json`，先运行 `/understand <repo> --language zh`，再重跑 pipeline。
