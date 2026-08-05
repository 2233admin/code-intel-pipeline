use std::env;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    English,
    Chinese,
}

impl Language {
    pub fn from_env() -> Self {
        match env::var("CODE_INTEL_LANG")
            .ok()
            .and_then(|v| v.to_lowercase().chars().next())
        {
            Some('z') | Some('c') => Language::Chinese,
            _ => Language::English,
        }
    }
}

pub struct Messages {
    lang: Language,
}

impl Messages {
    pub fn new(lang: Language) -> Self {
        Messages { lang }
    }

    pub fn ready(&self) -> &'static str {
        match self.lang {
            Language::English => "ready",
            Language::Chinese => "已就绪",
        }
    }

    pub fn consent_required(&self) -> &'static str {
        match self.lang {
            Language::English => "consent_required",
            Language::Chinese => "需要确认",
        }
    }

    pub fn provider_unavailable(&self) -> &'static str {
        match self.lang {
            Language::English => "Provider unavailable",
            Language::Chinese => "提供者不可用",
        }
    }

    pub fn model_unavailable(&self) -> &'static str {
        match self.lang {
            Language::English => "Model unavailable",
            Language::Chinese => "模型不可用",
        }
    }

    pub fn config_error(&self) -> &'static str {
        match self.lang {
            Language::English => "Configuration error",
            Language::Chinese => "配置错误",
        }
    }

    pub fn local_tool_error(&self) -> &'static str {
        match self.lang {
            Language::English => "Local tool error",
            Language::Chinese => "本地工具错误",
        }
    }

    pub fn endpoint_not_configured(&self) -> &'static str {
        match self.lang {
            Language::English => "Endpoint not configured",
            Language::Chinese => "端点未配置",
        }
    }

    pub fn authentication_absent(&self) -> &'static str {
        match self.lang {
            Language::English => "Authentication credentials absent",
            Language::Chinese => "认证凭证缺失",
        }
    }

    pub fn cc_switch_query_failed(&self) -> &'static str {
        match self.lang {
            Language::English => "CC Switch query failed",
            Language::Chinese => "CC Switch 查询失败",
        }
    }

    pub fn selected_channel(&self) -> &'static str {
        match self.lang {
            Language::English => "Selected channel",
            Language::Chinese => "已选通道",
        }
    }

    pub fn cost_scope(&self) -> &'static str {
        match self.lang {
            Language::English => "Cost scope",
            Language::Chinese => "成本范围",
        }
    }

    pub fn local_compute(&self) -> &'static str {
        match self.lang {
            Language::English => "Local compute",
            Language::Chinese => "本地计算",
        }
    }

    pub fn metered_api(&self) -> &'static str {
        match self.lang {
            Language::English => "Metered API",
            Language::Chinese => "按量计费 API",
        }
    }

    pub fn subscription(&self) -> &'static str {
        match self.lang {
            Language::English => "Subscription",
            Language::Chinese => "订阅",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn english_messages() {
        let msgs = Messages::new(Language::English);
        assert_eq!(msgs.ready(), "ready");
        assert_eq!(msgs.provider_unavailable(), "Provider unavailable");
    }

    #[test]
    fn chinese_messages() {
        let msgs = Messages::new(Language::Chinese);
        assert_eq!(msgs.ready(), "已就绪");
        assert_eq!(msgs.provider_unavailable(), "提供者不可用");
    }

    #[test]
    fn language_from_env() {
        std::env::set_var("CODE_INTEL_LANG", "zh");
        assert_eq!(Language::from_env(), Language::Chinese);
        std::env::remove_var("CODE_INTEL_LANG");
        assert_eq!(Language::from_env(), Language::English);
    }
}
