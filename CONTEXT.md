# Code Intel Pipeline

Code Intel Pipeline is an independent engineering-intelligence domain. It turns repository and delivery evidence into deterministic engineering facts, derived views, diagnoses, and plans that other products may consume without owning its language or runtime.

## Language

**Code Intel Pipeline**: The independent engineering-intelligence system that turns a Target Repository and its delivery evidence into deterministic engineering facts, derived views, diagnoses, and plans. OpenCLI Admin and other systems are consumers, not owners or internal modules of this domain.
_Avoid_: OpenCLI Admin feature, analyzer, crawler, generic scanner

**Agent Goal Intake**: The pre-scan task-contract layer that turns vague work into a bounded goal, verification evidence, constraints, iteration policy, stop conditions, and pause conditions.
_Avoid_: Scanner, prompt template, backlog item

**Agent Loop Pattern**: A repeatable agent execution shape such as goal-until-done, interval watch, or scheduled maintenance. It guides how an Agent keeps working but does not itself produce repository evidence.
_Avoid_: Pipeline run, artifact run, orchestration framework

**Harness Factory Reference**: A packaging and distribution reference for turning repository intelligence into a branded Agent harness, CLI, host configuration, release gates, and provenance. It is a future shipping pattern, not a scanner dependency.
_Avoid_: Scanner, Agent Goal Intake, runtime dependency

**Skill Development Benchmark**: A quality baseline for creating and hardening reusable skills, including trigger design, evaluation evidence, portability, release gates, failure cases, and review artifacts.
_Avoid_: Runtime dependency, scanner contract, prompt style guide

**Ponytail Value Filter**: The Agent-output discipline that first rejects code, abstractions, dependencies, files, tests, documentation, or process that does not need to exist, then chooses the first sufficient solution rung for work that remains necessary. It is never permission to skip understanding, evidence, safety, or verification.
_Avoid_: Code formatter, shortest-code contest, indiscriminate deletion, permission to under-build

**Necessity Trace**: The explicit link from an Agent-produced artifact to a current value source: an operator-requested outcome, Committed Engineering Plan deliverable, verified defect or risk, required contract or gate, evidence-closing spike, or approved debt reduction. Output without a Necessity Trace is rejected by the Ponytail Value Filter.
_Avoid_: Generic justification, future possibility, author preference

**Project Management Support**: agent-intake layer turning repository evidence into trackable work and durable project knowledge through issue tracker selection, triage labels, domain docs, and optional wiki surfaces.
_Avoid_: Scanner runtime, artifact producer, hidden credential store

**Linear Issue Tracker**: optional external project-management queue where verified Code Intel plans and artifact links can become actionable issues.
_Avoid_: Scanner dependency, default external write target, token storage

**Obsidian/LLM Wiki**: optional knowledge surface that mirrors, indexes, or links repo docs and artifact summaries for project navigation.
_Avoid_: Artifact authority, source of truth, replacement for ADRs or scanner evidence

**Target Repository**: The repository being examined by a pipeline run.
_Avoid_: Project, workspace, source tree

**Engineering Evidence Set**: The provenance-bearing, point-in-time collection of available engineering evidence for one analysis scope, anchored by a Target Repository and optionally including specifications, decisions, issue records, CI results, releases, runtime evidence, incidents, and ownership records. Every source declares freshness and availability so missing evidence remains unknown rather than being interpreted as absence.
_Avoid_: Repository snapshot, context dump, all project data, implicit complete picture

**Evidence Provider**: A replaceable external or built-in source that supplies Observed Evidence for one or more declared engineering capabilities. A provider does not become authoritative merely because it is installed or returns successfully.
_Avoid_: Pipeline dependency, source of truth, hardcoded tool

**Evidence Provider Port**: The Pipeline-owned contract describing the evidence, provenance, snapshot, freshness, completeness, and failure semantics required from an Evidence Provider. Providers remain independent and do not share Pipeline internals.
_Avoid_: Provider-native API, shared internal model, common database

**Provider Adapter**: The replaceable translation boundary that maps one provider's native protocol and evidence into an Evidence Provider Port without requiring either repository to depend on the other's internal implementation.
_Avoid_: Shared library coupling, database integration, provider fork

**Observed Evidence**: Provenance-bearing output captured from an Evidence Provider before Pipeline validation establishes its schema, snapshot, freshness, completeness, and admissibility as an Engineering Fact.
_Avoid_: Engineering Fact, trusted result, provider opinion

**Selective Internalization**: The policy of reusing mature Evidence Providers by default while bringing only indispensable engineering semantics, trust boundaries, and minimum survival capabilities under Pipeline ownership. Internalization is justified by authority, verifiability, portability, supply-chain, or shared-semantics needs rather than by a desire to eliminate dependencies.
_Avoid_: Reimplement every dependency, vendor everything, dependency minimization as a goal

**Engineering Capability Gap**: A required engineering ability that the current project, Pipeline, or available Evidence Providers cannot satisfy with admissible evidence and acceptable constraints.
_Avoid_: Missing package, implementation task, vague problem

**Engineering Assistance Discovery**: The evidence-driven process of finding internal atomic projects, external tools, established methods, and documentation sources that may resolve an Engineering Capability Gap.
_Avoid_: Package search, GitHub browsing, automatic dependency installation

**Solution Candidate Dossier**: A comparable evidence package for one possible response to an Engineering Capability Gap, covering fit, provenance, version, maintenance, license, security, compatibility, integration cost, reversibility, and validation status.
_Avoid_: Recommendation list, star ranking, LLM summary

**Adoption Decision**: The explicit decision to build, reuse, adapt, selectively internalize, reject, or defer a candidate after its Solution Candidate Dossier and validation evidence have been reviewed.
_Avoid_: Automatic install, tool recommendation, dependency detection

**Decision Gap**: A missing choice about intent, trade-offs, authority, priority, resources, or risk acceptance that cannot be resolved from Engineering Facts. It blocks only the decisions and plan branches that depend on it while unrelated deterministic analysis continues.
_Avoid_: Missing fact, generic question, reason to stop all work

**Decision Interview**: The one-question-at-a-time process by which an Agent closes a Decision Gap with a recommended answer, evidence, and consequences while retrieving discoverable facts instead of asking the operator for them.
_Avoid_: Questionnaire, requirements dump, asking the user to inspect the repository

**Decision Record**: The provenance-bearing result of a resolved Decision Gap, recording the accepted choice and enough context to prevent the same decision from being repeatedly reopened without new evidence.
_Avoid_: Chat transcript, inferred preference, undocumented approval

**Progressive Project Understanding**: The staged construction of a provenance-bearing project model, beginning with repository bootstrap and a fast Project Orientation, then deepening into structural and task-specific understanding only as needed.
_Avoid_: Full rescan before every task, LLM repository summary, one-shot context dump

**Project Orientation**: The first actionable view of a project, covering identity, purpose, language, major boundaries, entry points, commands, active change, available evidence, known risks, unknowns, and confidence. A typical repository should produce this view within sixty seconds without requiring an LLM.
_Avoid_: README summary, architecture deep dive, complete project understanding

**Project Understanding Quadrant**: The prioritization view that classifies project knowledge by System Criticality and Evidence Confidence into Known Core, Critical Unknown, Supporting Context, and Deferred Unknown. It directs limited understanding effort toward the project skeleton and the most consequential uncertainty first.
_Avoid_: Equal-depth repository reading, urgent-important task matrix, hidden unknowns

**Light-Speed Baseline**: The shortest credible path from an engineering problem to verified completion after preserving irreducible technical work, correctness, safety, and evidence gates. It exposes avoidable delay from queues, handoffs, repeated understanding, rework, and unnecessary coordination rather than rewarding speed that bypasses verification.
_Avoid_: Move fast and break things, shortest implementation time, skipping gates, raw Agent throughput

**Engineering Method Catalog**: The authoritative catalog of established engineering methods that Pipeline can select and apply from declared problem signals, required evidence, assumptions, deterministic steps, outputs, confidence rules, and execution cost. A method may have built-in or external implementations without changing its engineering meaning.
_Avoid_: Methodology wiki, prompt library, list of diagrams, LLM best practices

**Method Implementation**: A replaceable built-in or external implementation of an Engineering Method that declares compatibility, provenance, effects, and output conformance while leaving method selection and result authority with Pipeline.
_Avoid_: Method definition, hardcoded dependency, trusted tool by name

**Open-Source Reuse Ladder**: The ordered preference for adopting an open-source capability through invoke, adapt, depend, vendor, fork, port, or reimplementation, choosing the lowest ownership burden that satisfies the required trust and capability boundary.
_Avoid_: Copy first, dependency avoidance, default fork, language rewrite

**Reuse Record**: The provenance-bearing decision record for one reused capability, including source revision, license obligations, adoption mode, compatibility, maintenance and security evidence, owned modifications, and upgrade or exit strategy.
_Avoid_: Dependency entry, bookmark, attribution-only note

**Internalization Standard**: The tool-neutral governance lifecycle by which an external project, method, or idea becomes a Design Reference, Evidence Provider, Method Implementation, adapted capability, or selectively owned implementation. Every case uses the same evidence, adoption, contract, conformance, measurement, update, and retirement rules; OpenSpec, spec-kit, and creators may implement the lifecycle but never define its meaning.
_Avoid_: One policy per tool, OpenSpec-owned semantics, copy-and-document workflow, integration-specific standards

**Advisory Atom**: An independently executable capability that consumes Engineering Facts and Derived Engineering Models and returns a non-authoritative recommendation with evidence, confidence, and alternatives. It cannot create an Adoption Decision or Committed Engineering Plan.
_Avoid_: Policy gate, automatic installer, project authority, LLM opinion without evidence

**Workflow Recommender**: The tool-neutral Advisory Atom that evaluates project evidence and recommends applicable delivery, quality, specification, or engineering-method workflows. OpenSpec, spec-kit, gstack, and creator tools are candidates it may recommend, not dependencies embedded in the scanner core.
_Avoid_: OpenSpec detector, workflow installer, main runner logic, hardcoded preferred stack

**Engineering Fact**: An authoritative observation captured from a named evidence source, such as repository state, test output, CI state, runtime telemetry, or an approved project record. It carries provenance and is not created by interpretation alone.
_Avoid_: LLM conclusion, inferred opinion, undocumented assumption

**Derived Engineering Model**: A reproducible model computed from Engineering Facts, such as a dependency graph, critical path, traceability matrix, compatibility result, or risk metric. The same inputs and toolchain must produce the same model.
_Avoid_: Generated narrative, planning opinion, manually maintained diagram

**Engineering Plan Proposal**: A non-authoritative candidate ordering of engineering work, milestones, trade-offs, and verification conditions derived from Engineering Facts and Derived Engineering Models. Rules, tools, operators, or an LLM may propose it, but it creates no delivery commitment.
_Avoid_: Project commitment, approved roadmap, generated backlog as fact

**Committed Engineering Plan**: An explicitly approved engineering plan with accountable ownership, accepted priorities, dependencies, and verifiable completion conditions. It may originate from an Engineering Plan Proposal, but approval is the boundary that makes it authoritative.
_Avoid_: LLM plan, draft roadmap, inferred priority

**Scanner**: The authoritative producer of a new artifact run for a target repository. The scanner gathers current evidence and writes the reports that other components read.
_Avoid_: PowerShell runner, Rust scanner, artifact reader

**Resume CLI**: A command-line reader for existing artifact runs. It does not produce fresh repository evidence and does not replace the scanner.
_Avoid_: Rust scanner, new pipeline, replacement scanner

**Artifact Run**: The durable record produced by one scanner execution for one target repository. It is the unit that later tools resume from, classify, or hand to an Agent.
_Avoid_: Scan folder, output dump, report batch

**Capability Atom**: One independently executable responsibility with a versioned request, a versioned result, declared effects, and independently verifiable artifacts.
_Avoid_: Function, microservice, tiny script

**Snapshot Identity**: Portable identity of the repository inputs consumed by a Capability Atom: repository identity, HEAD, working-tree policy, scope, and input digest.
_Avoid_: Timestamp, checkout path, branch name

**Artifact Ref**: A typed reference carrying its own envelope version, the referenced payload schema, location, SHA-256 content identity, and consumed Snapshot Identity between Capability Atoms.
_Avoid_: File path, inline evidence dump, latest output

**Effect Boundary**: Pre-execution permission boundary plus post-execution audit for a Capability Atom. Determinism is separate from the allowed and observed effects: repository read, local write, network, or repository mutation.
_Avoid_: Tool type, permission prompt, implementation language

**Domain Verdict**: Evidence judgment returned by a completed capability: pass, fail, unknown, or not applicable. It is independent of process execution status.
_Avoid_: Exit code, exception, health score

**Run Commit**: Transactional publication boundary that promotes validated staged artifacts and writes `run-complete.json` last.
_Avoid_: Git commit, timestamp directory, successful subprocess

**Materialized View**: Rebuildable human or index projection derived from machine artifacts, such as summary Markdown or the cross-repository index.
_Avoid_: Source of truth, artifact producer, mutable task state

**Artifact Data Contract**: The stable meaning and ownership of generated artifact files and fields.
_Avoid_: JSON shape, output format, file schema

**Artifact Consumer**: A tool or Agent that reads existing artifact runs without producing new repository evidence.
_Avoid_: Scanner, producer, runner

**Code Evidence Layer**: Optional post-scan evidence layer that produces measurable code-structure artifacts such as symbols, chunks, relationships, and metrics. It supplements an Artifact Run but does not own scanner success semantics.
_Avoid_: Scanner, AST platform, semantic search tool

**CodeNexus Evidence Provider**: The independent code-and-Git intelligence provider that owns structural perception, indexing, retrieval, and impact relationships for source repositories. Code Intel Pipeline consumes its provenance-bearing evidence without owning its storage, runtime, or internal graph implementation.
_Avoid_: Pipeline module, shared database, embedded scanner, Pipeline source dependency

**Survival Scanner**: The smallest built-in repository scanner needed to identify a Target Repository, capture basic evidence, and diagnose unavailable Evidence Providers. It preserves Pipeline operability but does not compete with a specialized provider such as CodeNexus.
_Avoid_: Full CodeNexus replacement, duplicate intelligence platform, preferred scanner

**Agent Code Slice**: Curated, task-oriented view of Code Evidence Layer artifacts for Agent consumption. It points into full evidence dumps instead of replacing them.
_Avoid_: Full AST dump, summary, report

**Artifact Index**: A derived cross-repository view of artifact runs.
_Avoid_: Artifact run, cache, database

**Artifact Root**: The parent location that stores artifact runs across target repositories and sessions.
_Avoid_: Cache, temp directory, output folder

**Run Report**: The machine-facing summary of a scanner execution, including step outcomes, failure categories, and links to generated artifacts.
_Avoid_: Log, transcript

**Summary**: The human-facing entry point for a completed artifact run.
_Avoid_: Overview, readme

**Understanding**: The Agent handoff for a target repository. It tells the next Agent what to read and how to continue from the artifact run.
_Avoid_: Notes, memo, analysis

**Hospital Report**: The diagnosis layer for an artifact run. It names the current disposition, primary diagnosis, next protocol, and discharge criteria.
_Avoid_: Health report, QA report, status page

**Surgery Plan**: The first bounded repair plan selected from the current diagnosis and structural evidence.
_Avoid_: Refactor plan, todo list, cleanup list

**Next Protocol**: The next workflow state recommended by the hospital report.
_Avoid_: Next step, action, command

**GitHub Solution Research**: The evidence-gathering protocol for blocker categories where upstream issues, pull requests, repositories, or code examples may explain a solution.
_Avoid_: Web search, GitHub lookup, research mode

**Failure Category**: A normalized reason a scanner step could not produce clean evidence.
_Avoid_: Error type, exit code, exception

**Governed Scope**: A target repository or subpath with architecture rules and gate checks strong enough to support treatment decisions.
_Avoid_: Clean repo, healthy repo

**Target Scope**: The bounded subsystem inside a target repository selected for a governed scan.
_Avoid_: Folder, module, package

**Quant System Target**: A target repository where artifact evidence supports quantitative research, trading, or data-system development and must preserve lineage.
_Avoid_: Trading project, quant repo, data app

**Agent**: The consumer that reads artifact-run evidence before editing a target repository.
_Avoid_: Bot, assistant, worker
