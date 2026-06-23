# Code Intel Pipeline

Code Intel Pipeline is a local repository-intelligence workflow. Its language distinguishes the component that produces fresh evidence from the component that resumes from evidence already written.

## Language

**Code Intel Pipeline**: The workflow that turns a target repository into repository intelligence, diagnosis, and the next recommended protocol.
_Avoid_: Analyzer, crawler, generic scanner

**Agent Goal Intake**: The pre-scan task-contract layer that turns vague work into a bounded goal, verification evidence, constraints, iteration policy, stop conditions, and pause conditions.
_Avoid_: Scanner, prompt template, backlog item

**Agent Loop Pattern**: A repeatable agent execution shape such as goal-until-done, interval watch, or scheduled maintenance. It guides how an Agent keeps working but does not itself produce repository evidence.
_Avoid_: Pipeline run, artifact run, orchestration framework

**Harness Factory Reference**: A packaging and distribution reference for turning repository intelligence into a branded Agent harness, CLI, host configuration, release gates, and provenance. It is a future shipping pattern, not a scanner dependency.
_Avoid_: Scanner, Agent Goal Intake, runtime dependency

**Skill Development Benchmark**: A quality baseline for creating and hardening reusable skills, including trigger design, evaluation evidence, portability, release gates, failure cases, and review artifacts.
_Avoid_: Runtime dependency, scanner contract, prompt style guide

**Implementation Minimalism Benchmark**: An implementation-choice baseline requiring Agent code work to choose the smallest sufficient option before writing code: do nothing, reuse this repository, standard library, platform native capability, already-installed dependency, one-liner, then smallest local implementation.
_Avoid_: Runtime dependency, scanner contract, permission to skip evidence or safety

**Project Management Support**: agent-intake layer turning repository evidence into trackable work and durable project knowledge through issue tracker selection, triage labels, domain docs, and optional wiki surfaces.
_Avoid_: Scanner runtime, artifact producer, hidden credential store

**Linear Issue Tracker**: optional external project-management queue where verified Code Intel plans and artifact links can become actionable issues.
_Avoid_: Scanner dependency, default external write target, token storage

**Obsidian/LLM Wiki**: optional knowledge surface that mirrors, indexes, or links repo docs and artifact summaries for project navigation.
_Avoid_: Artifact authority, source of truth, replacement for ADRs or scanner evidence

**Target Repository**: The repository being examined by a pipeline run.
_Avoid_: Project, workspace, source tree

**Scanner**: The authoritative producer of a new artifact run for a target repository. The scanner gathers current evidence and writes the reports that other components read.
_Avoid_: PowerShell runner, Rust scanner, artifact reader

**Resume CLI**: A command-line reader for existing artifact runs. It does not produce fresh repository evidence and does not replace the scanner.
_Avoid_: Rust scanner, new pipeline, replacement scanner

**Artifact Run**: The durable record produced by one scanner execution for one target repository. It is the unit that later tools resume from, classify, or hand to an Agent.
_Avoid_: Scan folder, output dump, report batch

**Artifact Data Contract**: The stable meaning and ownership of generated artifact files and fields.
_Avoid_: JSON shape, output format, file schema

**Artifact Consumer**: A tool or Agent that reads existing artifact runs without producing new repository evidence.
_Avoid_: Scanner, producer, runner

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
