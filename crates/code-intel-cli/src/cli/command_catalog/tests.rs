use super::super::legacy::FULL_HELP_TEXT;
use super::super::primary::PrimaryArgs;
use super::contract::{
    AuthorityCondition, CommandAuthority, CommandEffect, ControllerOwnership, OutputContract,
};
use super::routes::{
    resolve_command_route, resolve_legacy_route, resolve_raw_route, CommandRoute, COMMAND_ROUTES,
};
use super::{
    execute_command, parse_command, render_outcome, Command, CompatibilityCommand,
    CompatibilityRoute, VersionCommand,
};

#[test]
fn parser_exposes_typed_commands_and_hides_route_slicing() {
    let version = parse_command(&["--version".into(), "--json".into()]).unwrap();
    assert!(matches!(
        version,
        Command::Version(VersionCommand { json: true })
    ));

    let primary = parse_command(&["--mode".into(), "lite".into()]).unwrap();
    assert!(matches!(
        primary,
        Command::Primary(PrimaryArgs { ref mode, .. }) if mode == "lite"
    ));

    let impact =
        parse_command(&["change".into(), "impact".into(), "--contract-probe".into()]).unwrap();
    assert!(matches!(
        impact,
        Command::Compatibility(CompatibilityCommand {
            route: CompatibilityRoute::ChangeImpact,
            ref arguments,
        }) if arguments == &["impact", "--contract-probe"]
    ));
}

#[test]
fn execute_and_render_buffer_version_output_at_the_typed_seam() {
    let command = parse_command(&["--version".into(), "--json".into()]).unwrap();
    let rendered = render_outcome(execute_command(command));

    assert_eq!(rendered.exit_code, 0);
    assert!(rendered.stderr.is_empty());
    assert!(rendered.stdout.ends_with('\n'));
    let value: serde_json::Value = serde_json::from_str(rendered.stdout.trim()).unwrap();
    assert_eq!(value["schema"], "code-intel-version.v1");
    assert_eq!(value["name"], "code-intel");
}

#[test]
fn render_maps_typed_primary_parse_errors_without_writing_during_parse() {
    let rendered = render_outcome(
        parse_command(&["--mode".into(), "invalid".into(), "--json".into()])
            .and_then(execute_command),
    );

    assert_eq!(rendered.exit_code, 64);
    assert!(rendered.stderr.is_empty());
    let value: serde_json::Value = serde_json::from_str(rendered.stdout.trim()).unwrap();
    assert_eq!(value["schema"], "code-intel-primary-result.v1");
    assert_eq!(value["outcome"], "error");
    assert_eq!(value["exitCode"], 64);
    assert_eq!(value["diagnostic"], "--mode must be lite, normal, or full");
}

#[test]
fn raw_routes_preserve_specific_dispatch_precedence_and_argument_offsets() {
    let decision_record = vec!["decision".into(), "record".into(), "--store".into()];
    let decision_default = vec!["decision".into(), "request-response".into()];
    let artifact_index = vec!["artifact".into(), "index".into(), "--repo".into()];
    let run_execute = vec!["run".into(), "execute".into(), "--repo".into()];
    let run_dag = vec!["run".into(), "dag-coordinate".into(), "--repo".into()];

    let record_route = resolve_raw_route(&decision_record).expect("decision record route");
    let default_route = resolve_raw_route(&decision_default).expect("decision default route");
    let artifact_route = resolve_raw_route(&artifact_index).expect("artifact index route");
    let execute_route = resolve_raw_route(&run_execute).expect("run execute route");
    let dag_route = resolve_raw_route(&run_dag).expect("run DAG route");

    assert_eq!(record_route.subcommand, Some("record"));
    assert_eq!(record_route.argument_offset, 1);
    assert_eq!(default_route.subcommand, None);
    assert_eq!(default_route.argument_offset, 1);
    assert_eq!(artifact_route.subcommand, Some("index"));
    assert_eq!(artifact_route.argument_offset, 1);
    assert_eq!(execute_route.id, CompatibilityRoute::RunExecute);
    assert_eq!(
        execute_route.contract.controller,
        ControllerOwnership::AuthoritativeRun
    );
    assert_eq!(
        execute_route.contract.authority,
        CommandAuthority::Committed
    );
    assert_eq!(dag_route.id, CompatibilityRoute::RunDagCoordinate);
    assert_eq!(dag_route.contract.controller, ControllerOwnership::Internal);
    assert_eq!(dag_route.contract.authority, CommandAuthority::Internal);
}

#[test]
fn invalid_run_namespace_is_a_parse_error_not_an_authority_route() {
    for raw in [vec!["run".into()], vec!["run".into(), "unknown".into()]] {
        assert!(resolve_raw_route(&raw).is_none());
        let rendered = render_outcome(parse_command(&raw).and_then(execute_command));
        assert_eq!(rendered.exit_code, 64);
        assert!(rendered.stdout.is_empty());
        assert_eq!(rendered.stderr, format!("{}\n", crate::run_cli::usage()));
    }
}

#[test]
fn legacy_parser_commands_are_not_intercepted_by_raw_routes() {
    let doctor = vec!["doctor".into(), "--json".into()];
    let provider_list = vec!["provider".into(), "--action".into(), "List".into()];

    assert!(resolve_raw_route(&doctor).is_none());
    assert!(resolve_raw_route(&provider_list).is_none());
}

#[test]
fn command_inventory_is_complete_unique_and_dispatchable() {
    let mut spellings = Vec::new();
    for command_route in COMMAND_ROUTES {
        match command_route {
            CommandRoute::Version(route) => {
                route.contract.assert_complete(route.command);
                for spelling in std::iter::once(route.command).chain(route.aliases.iter().copied())
                {
                    assert!(!spellings.contains(&spelling.to_string()));
                    spellings.push(spelling.to_string());
                    assert!(matches!(
                        resolve_command_route(&[spelling.to_string()]),
                        Some(CommandRoute::Version(_))
                    ));
                }
            }
            CommandRoute::Primary(contract) => {
                contract.assert_complete("<default-primary>");
                spellings.push("<default-primary>".into());
                assert!(matches!(
                    resolve_command_route(&[]),
                    Some(CommandRoute::Primary(_))
                ));
            }
            CommandRoute::Raw(route) => {
                let raw = match route.subcommand {
                    Some(subcommand) => vec![route.command.into(), subcommand.into()],
                    None => vec![route.command.into()],
                };
                let spelling = raw.join(" ");
                assert!(
                    !spellings.contains(&spelling),
                    "duplicate route: {spelling}"
                );
                spellings.push(spelling.clone());
                let resolved = resolve_raw_route(&raw).expect("registered raw route must resolve");
                assert_eq!(resolved.command, route.command);
                assert_eq!(resolved.subcommand, route.subcommand);
                assert_eq!(resolved.id, route.id);
                route.contract.assert_complete(&spelling);
            }
            CommandRoute::Legacy(route) => {
                route.contract.assert_complete(route.command);
                assert!(!spellings.contains(&route.command.to_string()));
                spellings.push(route.command.to_string());
                let resolved = resolve_legacy_route(route.command)
                    .expect("registered legacy command must resolve");
                assert_eq!(resolved.id, route.id);
                for spelling in route.aliases {
                    assert!(!spellings.contains(&spelling.to_string()));
                    spellings.push(spelling.to_string());
                    let resolved = resolve_legacy_route(spelling)
                        .expect("registered legacy alias must resolve");
                    assert_eq!(resolved.command, route.command);
                    assert_eq!(resolved.id, route.id);
                }
            }
        }
    }
}

#[test]
fn command_contracts_use_concrete_output_and_exit_types() {
    for route in COMMAND_ROUTES {
        let contract = match route {
            CommandRoute::Version(route) => &route.contract,
            CommandRoute::Primary(contract) => contract,
            CommandRoute::Raw(route) => &route.contract,
            CommandRoute::Legacy(route) => &route.contract,
        };
        contract.output_contract.assert_declared_formats();
        contract.exit_contract.assert_concrete();
    }
}

#[test]
fn unified_route_inventory_owns_version_primary_raw_and_legacy_dispatch() {
    assert!(COMMAND_ROUTES
        .iter()
        .any(|route| matches!(route, CommandRoute::Version(_))));
    assert!(COMMAND_ROUTES
        .iter()
        .any(|route| matches!(route, CommandRoute::Primary(_))));
    assert_eq!(
        COMMAND_ROUTES
            .iter()
            .filter(|route| matches!(route, CommandRoute::Raw(_)))
            .count(),
        31
    );
    assert_eq!(
        COMMAND_ROUTES
            .iter()
            .filter(|route| matches!(route, CommandRoute::Legacy(_)))
            .count(),
        12
    );
}

#[test]
fn command_authority_and_effect_contracts_cover_conditional_and_mutating_routes() {
    let raw = |command: &str, subcommand: Option<&str>| {
        COMMAND_ROUTES
            .iter()
            .find_map(|route| match route {
                CommandRoute::Raw(route)
                    if route.command == command && route.subcommand == subcommand =>
                {
                    Some(route)
                }
                _ => None,
            })
            .expect("registered raw command")
    };

    assert_eq!(
        raw("change", Some("impact")).contract.authority,
        CommandAuthority::Conditional(AuthorityCondition::CommittedOrStaleAdvisory)
    );
    assert_eq!(
        raw("artifact", Some("index")).contract.effects,
        &[CommandEffect::RepoRead, CommandEffect::LocalWrite]
    );
    assert_eq!(
        raw("artifact", Some("query")).contract.effects,
        &[CommandEffect::RepoRead, CommandEffect::ProcessSpawn]
    );
    assert_eq!(
        raw("repin", None).contract.effects,
        &[
            CommandEffect::RepoRead,
            CommandEffect::LocalWrite,
            CommandEffect::ProcessSpawn,
            CommandEffect::RepoMutation
        ]
    );
    assert_eq!(
        raw("model", None).contract.effects,
        &[CommandEffect::LocalWrite]
    );
    // `edit apply` is the only agent-facing route that rewrites source bytes.
    // Its declared effects must say so, and `edit impact` — the read-shaped
    // sibling one word away — must keep saying it does not.
    assert_eq!(
        raw("edit", Some("apply")).contract.effects,
        &[
            CommandEffect::RepoRead,
            CommandEffect::LocalWrite,
            CommandEffect::RepoMutation
        ]
    );
    assert_eq!(
        raw("edit", Some("impact")).contract.effects,
        &[CommandEffect::RepoRead, CommandEffect::ProcessSpawn]
    );
    for adapter in [
        "repowise-adapt",
        "graph-adapt",
        "sentrux-adapt",
        "codenexus-adapt",
    ] {
        assert_eq!(
            raw("provider", Some(adapter)).contract.effects,
            &[CommandEffect::RepoRead],
            "provider adapter effects: {adapter}"
        );
    }
    assert_eq!(
        raw("provider", Some("session-adapt")).contract.effects,
        &[CommandEffect::RepoRead, CommandEffect::LocalWrite],
        "session adapter can write its optional --out artifact"
    );
    let graph = COMMAND_ROUTES
        .iter()
        .find_map(|route| match route {
            CommandRoute::Legacy(route) if route.command == "graph" => Some(route),
            _ => None,
        })
        .expect("registered graph command");
    assert_eq!(
        graph.contract.effects,
        &[CommandEffect::RepoRead, CommandEffect::LocalWrite]
    );
}

#[test]
fn observable_contracts_pin_exact_schemas_composites_and_exit_sets() {
    let raw = |command: &str, subcommand: Option<&str>| {
        COMMAND_ROUTES
            .iter()
            .find_map(|route| match route {
                CommandRoute::Raw(route)
                    if route.command == command && route.subcommand == subcommand =>
                {
                    Some(&route.contract)
                }
                _ => None,
            })
            .expect("registered raw command")
    };
    let legacy = |command: &str| {
        COMMAND_ROUTES
            .iter()
            .find_map(|route| match route {
                CommandRoute::Legacy(route) if route.command == command => Some(&route.contract),
                _ => None,
            })
            .expect("registered legacy command")
    };

    assert_eq!(
        raw("snapshot", None).exit_contract.codes(),
        &[0, 64, 65, 69, 74]
    );
    assert_eq!(
        raw("model", None).exit_contract.codes(),
        &[0, 2, 64, 65, 74]
    );
    assert_eq!(
        raw("benchmark", None).exit_contract.codes(),
        &[0, 64, 65, 69, 70, 74]
    );
    assert_eq!(
        raw("repin", None).exit_contract.codes(),
        &[0, 1, 64, 65, 74]
    );
    assert_eq!(
        raw("evidence", None).exit_contract.codes(),
        &[0, 64, 65, 74]
    );
    assert_eq!(legacy("help").exit_contract.codes(), &[0, 1]);

    // Every exit `edit apply` declares has to answer on stdout, including the
    // ones reached before the capability envelope exists. Pinning the failure
    // identity next to the exit set is what keeps a later exit from being
    // added without a document to answer it with.
    assert_eq!(
        raw("edit", Some("apply")).exit_contract.codes(),
        &[0, 10, 64, 65, 69, 70, 74]
    );
    match raw("edit", Some("apply")).output_contract {
        OutputContract::Stdout { identities } => assert!(
            identities.contains(&"code-intel-edit-failure.v1"),
            "edit apply must declare the pre-envelope failure document: {identities:?}"
        ),
        other => panic!("edit apply has wrong output contract: {other:?}"),
    }

    for (adapter, outer_schema, nested_schema) in [
        (
            "repowise-adapt",
            "code-intel-repowise-route-result.v1",
            "code-intel-repowise-adapter-result.v1",
        ),
        (
            "graph-adapt",
            "code-intel-graph-route-result.v1",
            "code-intel-graph-adapter-result.v1",
        ),
        (
            "sentrux-adapt",
            "code-intel-sentrux-route-result.v1",
            "code-intel-sentrux-adapter-result.v1",
        ),
        (
            "codenexus-adapt",
            "code-intel-codenexus-route-result.v1",
            "code-intel-codenexus-adapter-result.v1",
        ),
    ] {
        match raw("provider", Some(adapter)).output_contract {
            OutputContract::Stdout { identities } => {
                assert_eq!(identities, &[outer_schema, nested_schema]);
            }
            other => panic!("provider adapter {adapter} has wrong output contract: {other:?}"),
        }
    }

    match raw("provider", Some("session-adapt")).output_contract {
        OutputContract::StdoutWithOptionalArtifactFiles {
            artifact_identities,
            stdout_identities,
        } => {
            assert_eq!(artifact_identities, &["code-intel-session-evidence.v1"]);
            assert_eq!(
                stdout_identities,
                &[
                    "code-intel-session-evidence.v1",
                    "code-intel-session-adapter-result.v1"
                ]
            );
        }
        other => panic!("session adapter has wrong output contract: {other:?}"),
    }
}

#[test]
fn full_help_documents_every_registered_raw_route() {
    for route in COMMAND_ROUTES.iter().filter_map(|route| match route {
        CommandRoute::Raw(route) => Some(route),
        _ => None,
    }) {
        let command = match route.subcommand {
            Some(subcommand) => format!("{} {subcommand}", route.command),
            None => route.command.to_string(),
        };
        assert!(
            FULL_HELP_TEXT.contains(&command),
            "FULL_HELP_TEXT is missing raw route: {command}"
        );
    }
}

#[test]
fn full_help_documents_every_registered_route_alias() {
    let registered_spellings = COMMAND_ROUTES.iter().filter_map(|route| match route {
        CommandRoute::Version(route) => Some((route.command, route.aliases)),
        CommandRoute::Legacy(route) => Some((route.command, route.aliases)),
        _ => None,
    });

    for (command, aliases) in registered_spellings {
        let spellings = std::iter::once(command)
            .chain(aliases.iter().copied())
            .collect::<Vec<_>>()
            .join("|");
        assert!(
            FULL_HELP_TEXT.contains(&format!("  {spellings}")),
            "FULL_HELP_TEXT is missing registered route spellings: {spellings}"
        );
    }
}

#[test]
fn full_help_alias_discoverability_has_a_v2_output_contract() {
    let help = COMMAND_ROUTES
        .iter()
        .find_map(|route| match route {
            CommandRoute::Legacy(route) if route.command == "help" => Some(route),
            _ => None,
        })
        .expect("registered help route");

    assert_eq!(
        help.contract.output_contract,
        OutputContract::Stdout {
            identities: &["text-format:help-quick.v1", "text-format:help-full.v2"]
        }
    );
}
