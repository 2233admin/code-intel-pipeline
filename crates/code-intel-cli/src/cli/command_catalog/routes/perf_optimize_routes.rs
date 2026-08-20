pub(super) const RUN: super::RawRoute = super::RawRoute {
    command: "perf-optimize",
    subcommand: Some("run"),
    argument_offset: 2,
    id: super::CompatibilityRoute::PerfOptimizeRun,
    contract: super::CommandContract {
        stability: super::CommandStability::Public,
        controller: super::ControllerOwnership::WorkspaceAdvisory,
        authority: super::CommandAuthority::Advisory,
        effects: &[
            super::CommandEffect::RepoRead,
            super::CommandEffect::ProcessSpawn,
        ],
        output_contract: super::OutputContract::Stdout {
            identities: &["code-intel-perf-optimize-run.v1"],
        },
        // 69 unavailable (no eval-command, eval-command unrunnable, or weco
        // missing/BYOK-unconfigured -- same three-way collapse
        // doctor_provider_rows.rs already uses); 76 the wall-clock budget
        // can't afford even one step (#157's precedent: too small for the
        // first unit of work is a failure, not a degraded success).
        exit_contract: super::ExitContract::Exact(&[0, 64, 69, 74, 76]),
        retirement_condition:
            "retain until #302 supersedes it with a candidate-safety-gated auto-PR flow that subsumes report-only runs",
    },
};

pub(super) const DENOISE_EVAL: super::RawRoute = super::RawRoute {
    command: "perf-optimize",
    subcommand: Some("denoise-eval"),
    argument_offset: 2,
    id: super::CompatibilityRoute::PerfOptimizeDenoiseEval,
    contract: super::CommandContract {
        stability: super::CommandStability::Internal,
        controller: super::ControllerOwnership::WorkspaceAdvisory,
        authority: super::CommandAuthority::Internal,
        effects: &[super::CommandEffect::ProcessSpawn],
        output_contract: super::OutputContract::Stdout {
            identities: &["text-format:denoise-eval-metric-line.v1"],
        },
        exit_contract: super::ExitContract::Exact(&[0, 64, 65]),
        retirement_condition:
            "internal plumbing `perf-optimize run` shells out to as weco's own --eval-command; retire only alongside RUN",
    },
};
