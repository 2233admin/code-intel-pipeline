# DR-0002 工具版本 pin 是下限，不是精确锁

Status: active
Date: 2026-08-08

## Decision

外部工具（repowise / rg / sentrux / …）的版本 pin 语义为**下限**：installed ≥ pin → 通过（`already_present`，detail 标注 `newer than pin X`）；installed < pin 或版本不可解析 → 才是 drift。禁止实现任何降级路径。

Owner 原话（2026-08-08）：「不是硬依赖，能有新的就肯定这套软件支持最新的。」

## Why

精确 pin 在真实机器上的行为：repowise 0.37.0（用户主动升级）被 0.36.0 精确 pin 判 `version_drift`，install check 报 FAILED，`-InstallMissing` 还会把它**降回** 0.36.0——每次重装都和用户对着干。supply-chain 关切（防过旧/未知版本）下限语义完全覆盖，精确锁提供的额外保证在这个项目的威胁模型里不存在。

反事实实证：这条口径没落账时，平行 session 产出了 #208——通篇论证「精确 pin 保供应链可复现」并把 pin 升成精确 0.38.0，与 owner 已定口径相反。决策不落账，蜂群就会替你反复重新决策。

## Enforcement

- `installer_version_gate.rs` `newer` 场景：装 0.37 pin 0.36 必须 `already_present`，installer block 被调用即 throw（防降级回归）
- 升 pin = 升下限：同步 `$script:RepowisePinnedVersion` 与 `orchestration/toolchain-versions.v1.json` 的 Repowise 下限（`comparison: minimum`）；缺失或过旧时仍精确获取已评审版本，已安装的更新版本不降级，无需动安装器语义。
- Repowise 的完整 PEP 440 报告与下限比较由 `code-intel repowise-version --reported "<report>" --minimum <floor>` 唯一拥有；兼容安装器只转发，不截断 rc/dev/post/local/epoch。重装后的版本也比较 `>= floor`，不恢复成字符串精确相等；打包安装拓扑下的 rc/post 与更新 post-release 复现进入 DR-0001 install-smoke CI。
