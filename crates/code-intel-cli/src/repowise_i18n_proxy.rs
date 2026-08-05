use serde_json::{json, Value};
use std::collections::HashMap;

pub struct RepowiseI18nProxy {
    translations: HashMap<String, HashMap<String, String>>,
}

impl RepowiseI18nProxy {
    pub fn new() -> Self {
        let mut translations = HashMap::new();

        let mut zh_cn = HashMap::new();
        zh_cn.insert("Dashboard".to_string(), "仪表盘".to_string());
        zh_cn.insert("Overview".to_string(), "概览".to_string());
        zh_cn.insert("System Map".to_string(), "系统地图".to_string());
        zh_cn.insert("Conformance".to_string(), "合规性".to_string());
        zh_cn.insert("Contracts".to_string(), "契约".to_string());
        zh_cn.insert("Co-Changes".to_string(), "共变".to_string());
        zh_cn.insert("Workspace".to_string(), "工作区".to_string());
        zh_cn.insert("Repositories".to_string(), "仓库".to_string());
        zh_cn.insert("Settings".to_string(), "设置".to_string());
        zh_cn.insert("Search".to_string(), "搜索".to_string());
        zh_cn.insert("Total Files".to_string(), "文件总数".to_string());
        zh_cn.insert("Total Symbols".to_string(), "符号总数".to_string());
        zh_cn.insert("Avg Coverage".to_string(), "平均覆盖".to_string());
        zh_cn.insert("Hotspots".to_string(), "热点".to_string());
        zh_cn.insert("Pages".to_string(), "页面".to_string());
        zh_cn.insert("Sync".to_string(), "同步".to_string());
        zh_cn.insert("Add Repository".to_string(), "添加仓库".to_string());
        zh_cn.insert("Help us improve Repowise".to_string(), "帮助我们改进 Repowise".to_string());
        zh_cn.insert("Light".to_string(), "浅色".to_string());
        zh_cn.insert("Dark".to_string(), "深色".to_string());
        zh_cn.insert("Theme preference".to_string(), "主题偏好".to_string());
        zh_cn.insert("Sync workspace".to_string(), "同步工作区".to_string());
        zh_cn.insert("docs skipped".to_string(), "文档跳过".to_string());
        zh_cn.insert("Needs attention".to_string(), "需要关注".to_string());
        zh_cn.insert("Cross-Repo Graph".to_string(), "跨仓库图".to_string());
        zh_cn.insert("Cross-Repo Intelligence".to_string(), "跨仓库智能".to_string());
        zh_cn.insert("Workspace Overview".to_string(), "工作区概览".to_string());
        zh_cn.insert("CO-CHANGE PAIRS".to_string(), "共变对".to_string());
        zh_cn.insert("PACKAGE DEPS".to_string(), "包依赖".to_string());
        zh_cn.insert("CONTRACT LINKS".to_string(), "契约链接".to_string());
        zh_cn.insert("CONTRACTS DETECTED".to_string(), "检测到的契约".to_string());
        zh_cn.insert("API Contracts".to_string(), "API 契约".to_string());
        zh_cn.insert("View all".to_string(), "查看全部".to_string());

        translations.insert("zh".to_string(), zh_cn);
        translations.insert("zh-CN".to_string(), translations.get("zh").unwrap().clone());
        translations.insert("zh-cn".to_string(), translations.get("zh").unwrap().clone());

        RepowiseI18nProxy { translations }
    }

    pub fn translate_response(&self, lang: &str, value: &Value) -> Value {
        if lang != "zh" && lang != "zh-CN" && lang != "zh-cn" {
            return value.clone();
        }

        let zh_map = match self.translations.get(lang) {
            Some(map) => map,
            None => return value.clone(),
        };

        let mut out = value.clone();
        Self::translate_in_place(&mut out, zh_map);
        out
    }

    /// Mutates in place instead of rebuilding the tree node-by-node: the
    /// workspace API returns hundreds of repo entries per response, and a
    /// clone-and-rebuild walk was allocating a fresh Value for every array
    /// and object on every request even though almost none of that content
    /// (repo names, paths) is ever in the translation table.
    fn translate_in_place(value: &mut Value, translations: &HashMap<String, String>) {
        match value {
            Value::String(s) => {
                if let Some(translated) = translations.get(s.as_str()) {
                    *s = translated.clone();
                }
            }
            Value::Object(obj) => {
                for (_, v) in obj.iter_mut() {
                    Self::translate_in_place(v, translations);
                }
            }
            Value::Array(arr) => {
                for v in arr.iter_mut() {
                    Self::translate_in_place(v, translations);
                }
            }
            _ => {}
        }
    }

    pub fn translate_html(&self, lang: &str, html: &str) -> String {
        if lang != "zh" && lang != "zh-CN" && lang != "zh-cn" {
            return html.to_string();
        }

        let zh_map = match self.translations.get(lang) {
            Some(map) => map,
            None => return html.to_string(),
        };

        // Repowise's UI ships as a client-rendered Next.js app: the raw HTML
        // this proxy fetches has no visible text yet, it only appears after
        // React hydrates in the browser. A one-shot string replace on the
        // server HTML can't reach that text, so translation instead runs
        // client-side via an injected MutationObserver that walks the live
        // DOM as React (re)renders it.
        let script = self.build_injection_script(zh_map);
        if let Some(pos) = html.rfind("</body>") {
            let mut result = String::with_capacity(html.len() + script.len());
            result.push_str(&html[..pos]);
            result.push_str(&script);
            result.push_str(&html[pos..]);
            result
        } else {
            format!("{}{}", html, script)
        }
    }

    fn build_injection_script(&self, zh_map: &HashMap<String, String>) -> String {
        let pairs: Vec<String> = zh_map
            .iter()
            .map(|(en, zh)| format!("[{},{}]", json!(en), json!(zh)))
            .collect();
        let table = pairs.join(",");

        format!(
            r#"<script id="repowise-i18n-injected">
(function() {{
  var DICT = new Map([{table}]);
  // Dedup by content, not by a per-node "done" marker: repowise's repo
  // list is a large virtualized list, so React recycles DOM nodes and
  // overwrites textContent directly on scroll. A node-identity marker
  // would survive the recycle and wrongly suppress re-translation of
  // reused rows; checking against the set of already-translated VALUES
  // is robust to that because it only looks at what the text currently is.
  var TRANSLATED_VALUES = new Set(DICT.values());

  function translateAttrs(el) {{
    if (!el.hasAttribute) return;
    ["title", "placeholder", "aria-label"].forEach(function(attr) {{
      var v = el.getAttribute(attr);
      if (v && DICT.has(v)) el.setAttribute(attr, DICT.get(v));
    }});
  }}

  function translateSubtree(node) {{
    if (node.nodeType === Node.TEXT_NODE) {{
      var text = node.nodeValue;
      var trimmed = text.trim();
      if (trimmed && !TRANSLATED_VALUES.has(trimmed) && DICT.has(trimmed)) {{
        node.nodeValue = text.replace(trimmed, DICT.get(trimmed));
      }}
      return;
    }}
    if (node.nodeType !== Node.ELEMENT_NODE) return;
    var tag = node.tagName;
    if (tag === "SCRIPT" || tag === "STYLE") return;
    translateAttrs(node);
    var child = node.firstChild;
    while (child) {{
      translateSubtree(child);
      child = child.nextSibling;
    }}
  }}

  // React fires many DOM mutations per tick (list scroll, re-render).
  // Batch them into one translation pass per animation frame instead of
  // walking a subtree synchronously for every single mutation record --
  // that per-mutation walk was the main source of input lag on click.
  var pending = [];
  var scheduled = false;
  function flush() {{
    scheduled = false;
    var batch = pending;
    pending = [];
    for (var i = 0; i < batch.length; i++) {{
      translateSubtree(batch[i]);
    }}
  }}
  function schedule(node) {{
    pending.push(node);
    if (!scheduled) {{
      scheduled = true;
      requestAnimationFrame(flush);
    }}
  }}

  var observer = new MutationObserver(function(mutations) {{
    for (var i = 0; i < mutations.length; i++) {{
      var m = mutations[i];
      if (m.type === "characterData") {{
        schedule(m.target);
      }} else {{
        for (var j = 0; j < m.addedNodes.length; j++) {{
          schedule(m.addedNodes[j]);
        }}
      }}
    }}
  }});

  function start() {{
    translateSubtree(document.body);
    observer.observe(document.body, {{ childList: true, subtree: true, characterData: true }});
  }}
  if (document.body) {{
    start();
  }} else {{
    document.addEventListener("DOMContentLoaded", start);
  }}
}})();
</script>"#,
            table = table
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translate_json_response() {
        let proxy = RepowiseI18nProxy::new();
        let input = json!({
            "title": "Dashboard",
            "items": [
                {"name": "Overview"},
                {"name": "System Map"}
            ]
        });

        let output = proxy.translate_response("zh", &input);
        assert_eq!(output["title"], "仪表盘");
        assert_eq!(output["items"][0]["name"], "概览");
        assert_eq!(output["items"][1]["name"], "系统地图");
    }

    #[test]
    fn translate_html_content() {
        let proxy = RepowiseI18nProxy::new();
        let html = "<button>Dashboard</button><span>Workspace</span>";
        let translated = proxy.translate_html("zh", html);
        assert!(translated.contains("仪表盘"));
        assert!(translated.contains("工作区"));
    }

    #[test]
    fn skip_non_chinese_lang() {
        let proxy = RepowiseI18nProxy::new();
        let input = json!({"title": "Dashboard"});
        let output = proxy.translate_response("en", &input);
        assert_eq!(output["title"], "Dashboard");
    }
}
