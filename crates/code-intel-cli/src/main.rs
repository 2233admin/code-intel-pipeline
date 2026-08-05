use std::env;
use std::io::{self, Write};
use std::process;

mod adapter_contract;
mod admissibility;
mod anchor_verification;
mod artifact_index;
mod artifact_ref;
mod artifacts;
mod audit_report;
mod authoritative_run;
mod authority;
mod capability;
mod capability_inventory;
mod change_agenda;
mod change_impact;
mod change_risk;
mod cli;
mod codenexus_adapter;
mod committed_evidence;
mod committed_evidence_controller;
mod compatibility_retirement_ticket;
mod dag_coordinator;
mod dag_run;
mod decision_port;
mod decision_record;
mod declared_pins;
mod doctor_bootstrap;
mod edit_apply;
mod edit_impact;
mod env_contract;
mod evidence_outcome;
mod evidence_query;
mod execution_policy;
mod file_boundary;
mod git_remote_registry;
mod graph;
mod hardened_git;
mod hospital_score;
mod i18n;
mod impact_graph;
mod invocation_identity;
mod language_pref;
mod mcp_serve;
mod method_catalog;
mod model_channels;
mod orchestration;
mod ponytail_gate;
mod project_orientation_benchmark;
mod providers;
mod repin;
mod repowise_hooks;
mod repowise_i18n_proxy;
mod repowise_proxy_server;
mod routes;
mod run_cli;
mod run_commit;
mod run_error;
mod runtime_ci_evidence;
mod sentrux;
mod sentrux_analysis;
mod sentrux_gate;
mod session_evidence;
mod snapshot;
mod stable_artifact;
mod staged_artifact;
mod survival_scan;
mod tool_effectiveness_benchmark;
mod workspace_advisory_controller;

#[cfg(test)]
mod phase4_authority_contract_tests;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn main() {
    let raw: Vec<String> = env::args().skip(1).collect();

    if raw.first().is_some_and(|arg| arg == "repowise-proxy") {
        let upstream_port: u16 = raw.get(1).and_then(|s| s.parse().ok()).unwrap_or(9000);
        let proxy_port: u16 = raw.get(2).and_then(|s| s.parse().ok()).unwrap_or(3000);
        let lang = env::var("CODE_INTEL_LANG").unwrap_or_else(|_| "en".to_string());

        repowise_proxy_server::start_proxy(upstream_port, proxy_port, &lang);
    }

    let rendered = cli::run(&raw);
    io::stdout()
        .write_all(rendered.stdout.as_bytes())
        .expect("write command stdout");
    io::stderr()
        .write_all(rendered.stderr.as_bytes())
        .expect("write command stderr");
    process::exit(rendered.exit_code);
}
