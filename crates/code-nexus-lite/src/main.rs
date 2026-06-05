//! # code-nexus-lite
//!
//! Rust binary that runs as an **iii worker** (Worker / Function / Trigger).
//! Wraps Repowise + Sentrux for cheap, Agent-friendly code-understanding context.
//!
//! ## 3 functions exposed
//!
//! - `codenexus::scan`   — call Repowise `init`/`augment` on a repo, write a graph snapshot
//! - `codenexus::lite`   — read a stored graph snapshot, return a compact Agent context
//! - `codenexus::doctor` — run Repowise/Sentrux/rg doctor checks, return JSON
//!
//! ## 3 HTTP triggers
//!
//! - POST /scan   → `codenexus::scan`
//! - POST /lite   → `codenexus::lite`
//! - POST /doctor → `codenexus::doctor`
//!
//! ## Why
//!
//! The PS1 `Invoke-CodeNexusLite.ps1` is Windows-only. This Rust binary
//! re-implements the same shape as an iii Rust worker, so the same Agent
//! can call it from Windows / Mac / Linux.
//!
//! ## Run
//!
//! ```bash
//! iii project init myapp    # one-time, scaffold an iii project
//! cd myapp && iii           # start the engine (default ws://127.0.0.1:49134)
//! cargo run --release       # in another shell, starts the worker
//! ```

use std::env;
use std::process;

use anyhow::{Context, Result};
use iii_sdk::{InitOptions, RegisterFunction, register_worker};
use serde_json::json;
use tracing::{error, info, warn};

mod functions;
mod triggers;

#[tokio::main]
async fn main() -> Result<()> {
    // Logging
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    // Engine URL (matches the iii CLI default)
    let engine_url =
        env::var("REMOTE_III_URL").unwrap_or_else(|_| "ws://127.0.0.1:49134".to_string());

    info!("code-nexus-lite v0.1.0 starting");
    info!("engine: {engine_url}");

    // Verify Repowise is on PATH (required for codenexus::scan and codenexus::doctor)
    if let Err(e) = functions::repowise::check_installed() {
        error!("repowise not found: {e}");
        error!("install: pip install repowise");
        process::exit(2);
    }

    // Register worker
    let iii = register_worker(&engine_url, InitOptions::default());

    // Register functions (Agent-callable units of work)
    // Note: 0.11.6 uses the new builder pattern — `iii.register_function(builder)` is single-arg,
    // and the function id lives inside the builder.
    iii.register_function(
        RegisterFunction::new_async("codenexus::scan", functions::scan).description(
            "Run Repowise on a repo, write a graph snapshot to .repowise/wiki.db",
        ),
    );
    iii.register_function(
        RegisterFunction::new_async("codenexus::lite", functions::lite).description(
            "Read a stored .repowise/wiki.db, return a compact Agent context (files + refs)",
        ),
    );
    iii.register_function(
        RegisterFunction::new_async("codenexus::doctor", functions::doctor)
            .description("Check Repowise / Sentrux / rg availability and return JSON"),
    );

    // Register HTTP triggers
    triggers::register_http_triggers(&iii).context("failed to register HTTP triggers")?;

    info!("registered functions: codenexus::scan / lite / doctor");
    info!("registered HTTP triggers: POST /scan /lite /doctor");
    info!("ready.  press Ctrl-C to exit.");

    // Block forever (the iii SDK runs its own background task on the worker handle)
    tokio::signal::ctrl_c().await?;
    warn!("Ctrl-C received, shutting down");
    Ok(())
}

/// Helper used by main and tests — never panics, returns Ok(json) on any failure path so
/// the Agent always gets a structured response.
#[allow(dead_code)]
fn version() -> serde_json::Value {
    json!({
        "name": "code-nexus-lite",
        "version": env!("CARGO_PKG_VERSION"),
        "engine": "iii-sdk",
        "license": "Apache-2.0",
    })
}
