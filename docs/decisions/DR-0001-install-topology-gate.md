# DR-0001 安装类 bug 的复现必须同 PR 进 install-smoke 闸

Status: active
Date: 2026-08-08

## Decision

任何「用户装不上 / 装上跑不了」类 bug 的修复 PR，必须同时把该 bug 的复现场景加进 install-path smoke 闸（#214 引入的 CI job）。只修代码不加复现的 PR 不算修完。

## Why

2026-08-08 实证：本仓 3794 个测试全部运行在 repo checkout 拓扑里，**没有一个运行在「装好的产品」拓扑里**。结果 v0.7.0 GA 携带至少四个安装即挂的缺陷发布：

- #218 安装器的 `<bin>/orchestration` 拷贝毒化 manifest 发现，doctor 必挂 ~40 条 entrypoint missing
- #216 sentrux-shim 挪到 `legacy/tools/` 后安装器 6 处引用没跟，forwarder 指向不存在路径，doctor `domain_failed`
- skill check `required=true` 写死，新机器默认安装必 FAILED
- README 声称 macOS/Linux 无 Release ZIP（实际三平台已发）

四个都活在「装好的二进制、新机器、无 checkout」拓扑里——恰好是所有闸都不踩的世界。修 #218 时连修复者本人都被抓：单测全绿，e2e sandbox 一跑才暴露第二份发现实现。**单测活在模块世界，安装 bug 活在拓扑世界；不进拓扑闸的修复会退化。**

## Enforcement

- install-smoke CI job（#214）是唯一合法宿主；复现进不去要在 PR 里写明原因
- 评审规约：安装类 fix PR 的 diff 里没有 workflow/smoke 变更即打回
- 首批回填：#218 bin-forwarder 拓扑、#216 shim 路径、skill-required 三个复现（见对应 PR）
