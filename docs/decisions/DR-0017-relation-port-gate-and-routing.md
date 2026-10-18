# DR-0017: Relation Port — external relations never decide gates, and producers merge per edge

Status: active
Date: 2026-10-18
Decision: Relations from the CodeNexus (CodeGraph engine) and LSP producers may drive impact answers, test selection and surgery localization, but gate verdicts (`sentrux gate`, `get_gate_verdict`, release gates) rest only on native, committed-run evidence. Relation producers are merged per edge rather than one being selected globally, and a producer becomes default only after a measured win.

## Authority and scope

The owner approved D1–D5 and the 2026-10-18 direction (CodeNexus uses CodeGraph as its engine; the pipeline drives language servers itself, reading OMP's `lsp.json`) in issue #439. Full design: [ADR-0015](../adr/0015-relation-port-and-relation-producers.md).

## Why

Without this record, a later session could promote external relations to gate authority, or let one producer replace another globally, and nothing would stop it. Both have concrete costs:
- CodeGraph evidence is `partial`. It does not parse this repository's PowerShell (`docs/research/codegraph-adoption.md`), so choosing it globally would silently drop those dependencies from impact and test selection.
- Promoting heuristic or AST relations into gate verdicts would let a parser gap change a pass/fail outcome, with no A05 fact-promotion rule to justify it.

## Enforcement

Convention only, until slice 4 of #439 lands the relation-port schema. From then on, schema and admission tests must reject:
- a gate verdict that cites non-native relations;
- a relation view built from a single producer when other fresh producers declared coverage for the same scope.

Changing either rule requires a new DR that supersedes this one.
