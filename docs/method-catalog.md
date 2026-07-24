# Method catalog contract

`method.catalog` is the C01 engineering-method vocabulary boundary. The checked catalog is
[`orchestration/methods/catalog.v1.json`](../orchestration/methods/catalog.v1.json), its closed
schema is [`code-intel-method-card.v1.schema.json`](../orchestration/schemas/code-intel-method-card.v1.schema.json),
and the independent loader is `crates/code-intel-cli/src/method_catalog.rs`.

The catalog records established method preconditions and repeatable transformations. It does not
select a method, claim that a method ran, execute a provider, or authorize an engineering decision.
Every card therefore carries `executionPolicy: catalog_only_no_selection_or_execution`; the
catalog manifest carries `selectionPolicy: none_catalog_only`.

## Validation invariants

- The manifest is the complete list of checked-in JSON cards and is strictly sorted by stable ID.
- Card and nested-object shapes are closed; missing required fields and unknown fields fail closed.
- IDs, card paths, described evidence, steps, outputs, ports, and confidence levels are unique.
- A step may reference declared evidence or an earlier step only, and every declared output must be
  produced. Related method IDs must resolve to another card.
- Source title/version/reference and explicit in-scope/out-of-scope boundaries are descriptive
  provenance, not proof that a method was executed.

The nine seed cards remain separate because their evidence models differ: causal observations,
failure modes, Boolean top events, activity networks, flow timestamps, improvement cycles,
time-ordered process samples, executable interface contracts, and migration seams are not
interchangeable inputs. C02 may later propose deterministic selection; C01 performs none.
