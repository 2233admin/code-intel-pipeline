//! HTTP triggers — bind each function to a POST endpoint via the iii SDK's
//! built-in `http` trigger type.
//!
//! Endpoints exposed:
//! - POST /scan   → codenexus::scan
//! - POST /lite   → codenexus::lite
//! - POST /doctor → codenexus::doctor

use anyhow::Result;
use iii_sdk::III;
use iii_sdk::builtin_triggers::{HttpMethod, HttpTriggerConfig};
use iii_sdk::RegisterTriggerInput;
use serde_json::json;
use tracing::info;

pub fn register_http_triggers(iii: &III) -> Result<()> {
    // POST /scan
    iii.register_trigger(RegisterTriggerInput {
        trigger_type: "http".to_string(),
        function_id: "codenexus::scan".to_string(),
        config: json!(HttpTriggerConfig::new("/scan").method(HttpMethod::Post)),
        metadata: None,
    })
    .map_err(|e| anyhow::anyhow!("failed to register /scan trigger: {e}"))?;
    info!("registered HTTP POST /scan → codenexus::scan");

    // POST /lite
    iii.register_trigger(RegisterTriggerInput {
        trigger_type: "http".to_string(),
        function_id: "codenexus::lite".to_string(),
        config: json!(HttpTriggerConfig::new("/lite").method(HttpMethod::Post)),
        metadata: None,
    })
    .map_err(|e| anyhow::anyhow!("failed to register /lite trigger: {e}"))?;
    info!("registered HTTP POST /lite → codenexus::lite");

    // POST /doctor
    iii.register_trigger(RegisterTriggerInput {
        trigger_type: "http".to_string(),
        function_id: "codenexus::doctor".to_string(),
        config: json!(HttpTriggerConfig::new("/doctor").method(HttpMethod::Post)),
        metadata: None,
    })
    .map_err(|e| anyhow::anyhow!("failed to register /doctor trigger: {e}"))?;
    info!("registered HTTP POST /doctor → codenexus::doctor");

    Ok(())
}
