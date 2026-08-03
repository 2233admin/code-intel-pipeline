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

        self.translate_value(value, zh_map)
    }

    fn translate_value(&self, value: &Value, translations: &HashMap<String, String>) -> Value {
        match value {
            Value::String(s) => {
                if let Some(translated) = translations.get(s) {
                    Value::String(translated.clone())
                } else {
                    Value::String(s.clone())
                }
            }
            Value::Object(obj) => {
                let mut new_obj = serde_json::Map::new();
                for (k, v) in obj {
                    new_obj.insert(k.clone(), self.translate_value(v, translations));
                }
                Value::Object(new_obj)
            }
            Value::Array(arr) => {
                Value::Array(
                    arr.iter()
                        .map(|v| self.translate_value(v, translations))
                        .collect(),
                )
            }
            _ => value.clone(),
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

        let mut result = html.to_string();
        for (en, zh) in zh_map {
            result = result.replace(en, zh);
        }
        result
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
