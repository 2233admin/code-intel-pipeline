# DR-0003 manifest 发现优先级与 probe 语义

Status: active
Date: 2026-08-08

## Decision

`orchestration/integrations.json` 的发现分三档，语义各不同：

1. **显式配置**（`--manifest` 参数、`CODE_INTEL_INTEGRATIONS_MANIFEST`）：无条件采信，**不做 probe**——用户点名的路径错了就报错，不静默换路径。
2. **猜测候选**（exe 祖先、cwd 祖先）：命中后必须过 entrypoint probe（`manifest_entrypoints_resolve`：manifest 的文件型 entrypoint 至少一个能在隐含根下解析），不过则弃用继续走。
3. **兜底**（`CODE_INTEL_HOME`）：安装器必写，猜测层全灭后落这里。

probe 判据是「**manifest 在这个根下真的能用**」（entrypoint 解析），不是「根长得像 checkout」（`.git`/`Cargo.toml` 标记）。manifest 与 root 必须同址——不允许「manifest 取 A 处拷贝、root 另指 B」的分家方案，两边内容会漂移。

所有发现实现共享同一个 probe 函数，禁止再造副本。

## Why

#218：安装器拷到 `<bin>/orchestration` 的 forwarder 被猜测层无条件采信，`<bin>` 被当仓库根，40 条 entrypoint missing，v0.7.0 干净安装必挂。修复时发现发现逻辑存在**两份独立实现**（`capability::discover_manifest` 与 `orchestration::resolve_manifest_path`，后者连 env var 都不读）——只修一份被另一份绕过，单测全绿、e2e 才抓到。writer/validator 各持一份清单的病型第 N 例（#184/#206/#226 同族）。

标记检查（`is_repo_like`）被否的原因：验的是形态不是不变量——带 `Cargo.toml` 的无关 checkout 会通过，缺标记的合法 release 根会被拒；且它允许 manifest 与 root 分家（#227 原方案），埋内容漂移雷。

## Enforcement

- `capability.rs` probe 单测 4 例（forwarder 拒收 / 可解析根通过 / 无文件型 entrypoint 不误杀 / 坏 JSON 拒收）
- install-smoke 闸的 bin-forwarder 拓扑复现（DR-0001 首批回填）
- 新增发现路径必须复用 `capability::manifest_entrypoints_resolve`，评审规约
