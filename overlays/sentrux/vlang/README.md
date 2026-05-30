# Sentrux V 覆盖包

Sentrux 0.5.7 自带的 `vlang` 标准包在 Windows 上不完整：`plugin.toml` 缺少 `[grammar]`，也没有 `grammars/windows-x86_64.dll`。`sentrux plugin add vlang` 和 `add-standard` 会反复装回这个坏包。

这个覆盖包补齐三件事：

- `plugin.toml`：声明 `tree-sitter-v` grammar 和 ABI 13。
- `grammars/windows-x86_64.dll`：由 `nedpals/tree-sitter-v` 编译，并导出 Sentrux 期望的 `tree_sitter_vlang()`。
- `queries/tags.scm`：改成该 grammar 真实存在的节点，并使用 Sentrux 能建图的 `@definition.*`、`@reference.call`、`@import.module` 捕获协议。

安装：

```powershell
.\Install-SentruxVlangOverlay.ps1
```

验证：

```powershell
sentrux plugin validate $env:USERPROFILE\.sentrux\plugins\vlang
sentrux plugin list
sentrux check C:\tmp\sentrux-vlang-fixture
```

注意：`sentrux scan <path>` 是打开 GUI 的命令，不适合作为非交互 smoke test。用 `check` 或 `gate` 验证结构引擎。

构建 DLL 的来源：

```powershell
git clone --depth=1 https://github.com/nedpals/tree-sitter-v.git C:\tmp\tree-sitter-v
gcc -shared -O2 -I C:\tmp\tree-sitter-v\src -o C:\tmp\tree-sitter-v\windows-x86_64.dll C:\tmp\tree-sitter-v\src\parser.c C:\tmp\tree-sitter-v\src\scanner.c C:\tmp\tree-sitter-v\sentrux_vlang_alias.c
```

`sentrux_vlang_alias.c` 只做一件事：把 grammar 原生导出的 `tree_sitter_v()` 包装成 Sentrux 查找的 `tree_sitter_vlang()`。
