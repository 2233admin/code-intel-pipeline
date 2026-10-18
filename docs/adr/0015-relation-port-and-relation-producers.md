---
status: accepted
date: 2026-10-18
---

# Relation Port: one relation view, three producers

Code Intel answers "what does this change touch" from a file-level import list.
`change impact`, `edit impact` and the MCP `get_change_impact` tool walk
`impact_graph::reverse_import_graph` over the imports that Native Code Evidence
extracts line by line. Surgery-plan localization does not use any relation graph:
`hospital_diagnosis` picks the target from structural violations or the hotspot
`top_target`. The model has two known limits. It has
no symbols or calls. Its resolution is weak outside path-is-module languages:
#219 measured 141 of 1988 Rust imports resolved, and #225 shows that Rust, Java, C#
and Go share one root cause. Four independent import/symbol extractors carry the
same heuristics: `native_code_evidence.rs`, `graph/mod.rs`, `sentrux_analysis.rs`
and `sentrux_quality_signal.rs`. A fix in one has repeatedly left the others
wrong; #437/#438 is the latest example.

CodeGraph (#429, merged in #434) already produces a tree-sitter symbol and call
graph, but only as a side query (`provider codegraph`, MCP `get_code_context`)
that no consumer reads. CodeNexus, the code-and-Git provider named in CONTEXT.md,
covers the same ground and is still pre-MVP.

## Decision

1. **One provider-neutral relation view.** Code Intel owns a
   `code-intel-relation-port.v1` artifact. The relation kinds are a closed set:
   `contains | calls | references | imports | implements | extends`. Confidence is
   categorical, not numeric. From strongest to weakest:
   `lsp_resolved > ast_extracted > name_inferred > heuristic`. Each relation
   carries `producer`, `producerVersion` and `consumedSnapshotIdentity`. A weaker
   producer never overwrites a stronger one for the same edge. This takes the #265
   design and makes it the contract that downstream consumers read.
2. **Three producers behind the port, no consumer-specific paths.**
   - *Native* (internal, always available): the four extractors are merged into
     one. It promises survival-level `heuristic` / `name_inferred` relations only.
     Language-specific resolution special cases are deleted once the benchmark in
     point 6 passes, not maintained in parallel.
   - *CodeNexus* (external): CodeNexus and CodeGraph fill one provider slot.
     CodeNexus uses CodeGraph as its parsing and symbol-graph engine and adds the
     Git overlay (blame, history, hotspots, co-change). The pipeline sees one
     adapter producing `ast_extracted` relations plus Git signals.
     `provider.codegraph` and `provider.codenexus-adapt` merge into that adapter.
     The demoted `crates/code-nexus-lite` incubator, which is not on any production
     path, is removed. The `legacy/Invoke-CodeNexusLite.ps1` compatibility route
     follows the existing E04 retirement process
     (`docs/compatibility-retire-codenexus-direct-branch.md`). It is deleted only
     after the normal facade uses B04/B05, the observation window or a DR-0006
     static-unreachability proof is satisfied, and E00 approval exists. Historical
     rollback fixtures are kept.
   - *LSP* (internal client, external servers): Code Intel drives language servers
     directly from Rust for definition, references, implementation and call
     hierarchy, producing `lsp_resolved` relations. The server list and settings
     are read from the same `lsp.json` locations OMP uses (`.omp/lsp.json`,
     `~/.omp/agent/lsp.json`, repository-root `lsp.json`), detected by root markers.
     Servers are never auto-installed. Code Intel does not depend on the OMP
     runtime: OMP exposes LSP only inside a model session, which is
     non-deterministic and costs a model turn.
3. **One traversal layer.** `impact_graph.rs` reads the relation view;
   `change impact`, `edit impact` and MCP `get_change_impact` go through it. Surgery
   planning is a new integration, not a migration: `hospital_diagnosis` keeps its
   target selection and gains the target's relation neighbourhood (callers,
   callees, affected tests) from the same layer. `contains` is not a dependency
   edge.
4. **Merge per edge, with a receipt.** Producers are not selected globally.
   Every fresh producer declares its coverage (languages and files it actually
   parsed), and the relation view is the union of their relations. The confidence
   order applies only when two producers report the same edge. Coverage gaps stay
   visible: for example, CodeGraph does not parse this repository's PowerShell,
   so those edges come from the native producer. A producer whose snapshot
   identity does not match the examined repository is stale and contributes
   nothing. Every answer records, per edge, which producer it came from, plus the
   coverage gaps. This supersedes the
   `graph-provider-adapter` rule that external execution is never an automatic
   primary path: a fresh, admitted external producer may contribute relations
   automatically, and each one is labelled with its producer.
5. **Gate authority does not move.** Relations from the CodeNexus and LSP
   producers may drive impact answers, test selection and localization at
   `ast_extracted` or stronger. Gate verdicts (`sentrux gate`, `get_gate_verdict`,
   release gates) still rest only on native, committed-run evidence. External
   relations stay advisory there until a measured win justifies an A05
   fact-promotion rule.
6. **Default switch is measured.** Native, CodeNexus and LSP are compared on
   this repository, k-atana and tdxcli-rs: impact recall and precision, selected
   test hit rate, and agent tool-call count. A producer becomes the default only
   after it wins, as #265 already requires.

## Host integration

Hosts stay neutral. The root `.mcp.json`, root `AGENTS.md` and `skills/` work
unchanged in Claude Code, Codex and OMP. OMP gets optional extras (#447): an
extension that runs the gate and change impact after edits, TTSR stream rules for
write-time constraints, and marketplace packaging. Shared `lsp.json` is the only
coupling to OMP, and it is a configuration file, not a runtime dependency.

## Consequences

- ADR-0007's "no complete call graph in v1" boundary is superseded for relations:
  calls become first-class relations with explicit confidence, while the native
  producer still makes no completeness claim.
- Node becomes an optional runtime: CodeGraph needs it. Language servers are
  optional binaries. Without them, `doctor` reports degraded relations, not
  failure, and the native producer still answers.
- #219–#225 and #297 are resolved by replacement, not by extending the per-language
  resolver. They close once slice 7 of #439 lands.
- The second consumer of the relation seam (#211's test for a real seam) is
  satisfied from slice 5 onward.

Implementation slices and their order live in #439.
