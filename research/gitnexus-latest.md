# GitNexus 最近更新与本仓库相关性

调查日期：2026-09-04

假设用户所说的 giteNexus 是上游项目 [abhigyanpatwari/GitNexus](https://github.com/abhigyanpatwari/GitNexus)。

## 上游状态

- 稳定线仍是 `v1.6.10`，发布日期为 2026-08-27。官方变更集中在解析正确性、Spring/JVM 建模、AST receiver-chain typing、索引稳定性、Windows 路径、FTS/embedding 恢复、大仓库稳定性和 MCP 安全边界。
- GitHub Releases 页面在本次查询中显示最新候选为 `v1.6.11-rc.47`，发布日期为 2026-09-03；它是 prerelease，不应直接替换稳定线。候选线近期加入或修正 Zig、增量 watch、跨仓 GraphQL contracts、Kotlin/Spring route/config consumer、远程 clone/analyze 同步、`grep`/`impact` 路径处理、watcher 重挂载和 bearer auth。
- 上游最近的方向不是简单增加一个代码文件摘要，而是把代码图、增量索引、跨仓契约、agent/MCP 接口和运行时可靠性一起做强。

## 对本仓库的比对

本仓库 issue #337 的 Rust 端口明确只覆盖旧 `Invoke-CodeNexusLite.ps1` 在生产调用点真正可达的路径：最大代码文件 fallback、有限文本引用、无 DSM/hotspot 输入、无 git history。它不等价于 GitNexus 的完整语义图，也不应声称支持 Spring/JVM/Zig/GraphQL/watch 等能力。

本次复现出一个真实兼容性 bug：旧 facade 在 `Resolve-Directory` 后使用规范化的 repo/target 路径，Rust CLI 之前直接使用用户传入的 `src/..` 等非规范路径，导致输出文件路径出现 `src/../src/...`；Windows `fs::canonicalize` 还会产生 `\\?\\` 扩展前缀，污染 references。现已在 `codenexus_generate.rs` 统一规范化目录并去除 Windows 扩展前缀，集成回归测试覆盖该路径。

## 建议

1. **必须学习**：优先吸收上游已经反复修复、且与本仓库直接重叠的可靠性原则——Windows/跨平台路径归一化、增量/并发写入的原子性、bounded output、MCP fail-closed 与 allowlist、解析失败和 degraded link 的诚实状态。
2. **已修复**：CodeNexus-Lite 的非规范 repo/target 路径兼容差异已修正；确定性基准两次验证均为 `6/6`，generated-path leak 为 `0`。
3. **继续保持范围**：只有在 facade parity 或 install-smoke 重现具体差异时才继续修；不要用上游新能力替换 #337 的兼容端口范围。
4. **暂不移植**：Spring/JVM、Zig、GraphQL、watch、remote sync、embedding 等完整 GitNexus 能力。它们属于新的语义/运行时范围，不是 #337 的兼容端口。
5. 稳定部署继续跟 `gitnexus@latest`；只在隔离测试中试 `gitnexus@rc`，并在重新索引后比较输出和失败状态。

## 官方来源

- [GitNexus CHANGELOG.md](https://github.com/abhigyanpatwari/GitNexus/blob/main/gitnexus/CHANGELOG.md)
- [GitNexus Releases](https://github.com/abhigyanpatwari/GitNexus/releases)
- [GitNexus main commits](https://github.com/abhigyanpatwari/GitNexus/commits/main)
- [v1.6.10 到 v1.6.11 RC 对比](https://github.com/abhigyanpatwari/GitNexus/compare/v1.6.10...v1.6.11-rc.47)
