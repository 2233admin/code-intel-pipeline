# Repowise 中文支持

Code Intel Pipeline 现在包含 Repowise UI 的中文翻译层。

## 快速开始

启动 Repowise 并启用中文：

```bash
cd D:\projects\code-intel-pipeline
repowise serve --lang zh
```

或通过浏览器查询参数：

```
http://localhost:3000?lang=zh
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

编辑 `src/repowise_i18n_proxy.rs` 添加更多词汇：

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
