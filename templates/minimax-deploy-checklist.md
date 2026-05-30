# MiniMax / Agent 部署模板

目标：在一台新 Windows 机器上，把 Code Intel Pipeline 跑到可用状态。

## 输入

- Pipeline 仓库：`https://github.com/2233admin/code-intel-pipeline`
- 目标项目路径：`<REPO_PATH>`
- 模式：`normal`

## 命令

```powershell
git clone https://github.com/2233admin/code-intel-pipeline.git
cd code-intel-pipeline
.\bootstrap-new-machine.ps1 -RepoPath <REPO_PATH>
```

## 验收

- `bootstrap-new-machine.ps1` 返回 `Code intel bootstrap: OK`
- `check-code-intel-tools.ps1 -RepoPath <REPO_PATH>` 返回 OK
- 最新 artifact 目录里存在：
  - `summary.md`
  - `report.json`
  - `understanding.md`
  - `sentrux-dsm.json`
  - `sentrux-file-details.json`
  - `sentrux-hotspots.json`
  - `sentrux-evolution.json`
  - `sentrux-what-if.json`
  - `codenexus-context.json`

## 如果失败

- 先读 `%LOCALAPPDATA%\code-intel\bootstrap\bootstrap-*.md`
- 再读 smoke artifact 里的 `summary.md`
- 如果缺真实 `sentrux.exe`，不用先修；shim 会启用 lite core
- 如果缺 Understand Anything，先接受 manual step，再按报告里的 `/understand ...` 命令补图谱
- 如果缺 provider key，只影响 repowise docs，不影响基础结构扫描
