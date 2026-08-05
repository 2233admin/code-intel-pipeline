# Repowise 中文支持

Code Intel Pipeline 现在包含 Repowise UI 的中文翻译层。

## 快速开始

中文翻译是由 `code-intel repowise-proxy` 反向代理完成的（实现见
`crates/code-intel-cli/src/repowise_proxy_server.rs`），代理进程在**启动时**从自身
的进程环境读取一次 `CODE_INTEL_LANG`，之后每个请求都复用这个值——**没有**按请求切
换语言的查询参数，`repowise serve` 本身也没有 `--lang` 这个参数。

1. 启动 Repowise 本体（监听其自己的端口；代理默认转发到 9000，端口不同请自行调整）：

```bash
repowise serve
```

2. 另开一个终端，在启动代理**之前**设置 `CODE_INTEL_LANG=zh`，再启动
   `code-intel repowise-proxy <上游端口> <代理端口>`（两个端口都可省略，默认分别是
   9000 和 3000）：

```bash
CODE_INTEL_LANG=zh code-intel repowise-proxy 9000 3000
```

3. 用浏览器访问**代理**监听的端口（不是 repowise 自己的端口）：

```text
http://localhost:3000
```

## 已翻译组件

**导航菜单：**
- Dashboard → 仪表盘
- Overview → 概览
- System Map → 系统地图
- Conformance → 合规性
- Contracts → 契约
- Co-Changes → 共变
- Workspace → 工作区
- Repositories → 仓库
- Settings → 设置

**UI 元素：**
- Total Files → 文件总数
- Total Symbols → 符号总数
- Avg Coverage → 平均覆盖
- Hotspots → 热点
- Pages → 页面
- Sync → 同步
- Light/Dark → 浅色/深色

**操作：**
- Add Repository → 添加仓库
- Sync workspace → 同步工作区
- Help us improve Repowise → 帮助我们改进 Repowise

## 实现方式

通过 `RepowiseI18nProxy` 拦截并翻译：
- JSON API 响应
- HTML 页面内容

不修改 Repowise 源码，无需重新编译。

## 使用场景

### 1. API 响应翻译

```rust
let proxy = RepowiseI18nProxy::new();
let response = fetch_repowise_api(endpoint);
let translated = proxy.translate_response("zh", &response);
```

### 2. HTML 页面翻译

```rust
let proxy = RepowiseI18nProxy::new();
let html = fetch_repowise_page();
let translated = proxy.translate_html("zh", &html);
```

## 扩展翻译

编辑 `crates/code-intel-cli/src/repowise_i18n_proxy.rs`（相对仓库根目录的路径）添加更多词汇：

```rust
zh_cn.insert("New Feature".to_string(), "新功能".to_string());
```

然后重新编译：
```bash
cargo build --release
```

## 支持的语言代码

- `zh`, `zh-CN`, `zh-cn` → 简体中文

## 限制

- 仅翻译预定义的 UI 文本
- 代码符号、仓库名称、作者名等保持原样
- 实时内容（文件内容、提交信息）不翻译

## 下一步

可添加支持：
- 日语（ja）
- 西班牙语（es）
- 法语（fr）

复制翻译映射并本地化即可。
