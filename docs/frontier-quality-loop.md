# Frontier-inspired quality loop

This note selectively absorbs four method-level ideas from
[`apoorvjain25/frontier`](https://github.com/apoorvjain25/frontier) at revision
`0b39eee3ec9ed0905ad7303772653c7e9bd17831` (MIT). It does not vendor the Frontier
skill, its agents, or its 21 domain craft standards.

## Owned semantics

1. **Evidence-bound findings.** A review finding names a concrete location, the violated
   criterion, the observed failure, and confidence. Missing evidence is reported as
   `unverified`; it is not silently converted to a pass.
2. **Author/reviewer separation.** The reviewer reports defects and does not repair them in
   the same pass. For material changes, the reviewer should not be the implementation author.
3. **Earned stop condition.** Completion is based on fresh validation evidence and no open
   required findings, not on reaching the end of a generation pass. Repeated whole-artifact
   sweeps are optional and risk-sized; they are never a reason for an unbounded loop.
4. **Rule-candidate distillation.** A repeated or high-value judgment may emit a proposed
   reusable rule. A proposal is advisory until a maintainer accepts it, adds a counterexample
   or fixture, chooses its scope, and supplies a verification command.

## Pipeline mapping

| Absorbed idea | Existing Pipeline owner | Local form |
| --- | --- | --- |
| Evidence-bound findings | artifact contract and hospital diagnosis | location + criterion + observation + confidence + evidence state |
| Author/reviewer separation | execution plans and independent verifier conditions | implementer and verifier remain distinct for material changes |
| Earned stop | completion gate and run-commit evidence | fresh targeted checks, explicit gaps, no required work pending |
| Rule distillation | Sentrux rules, method cards, skill benchmarks | candidate -> maintainer decision -> fixture -> governed rule |

## Distillation candidate shape

```text
RULE_CANDIDATE:
- scope: <artifact type, language, subsystem, or workflow>
- trigger: <observed repeated failure or high-value judgment>
- rule: <one checkable sentence>
- replacement: <preferred behavior, when the rule bans something>
- evidence: <artifact refs, file locations, or command output>
- counterexample: <valid case that must continue to pass>
- verification: <command or deterministic inspection>
- status: proposed
```

Candidates must not mutate `.sentrux/rules.toml`, skill instructions, templates, or production
policy automatically. Promotion is a separate authority-bearing change with regression evidence.

## Deliberately not absorbed

- Best-of-N generation as a default: expensive and unrelated to repository-understanding runs.
- Two consecutive clean sweeps as a universal gate: useful for high-risk deliverables, excessive
  for deterministic scanner output that already has targeted tests and schema validation.
- A taste gate in the scanner runtime: taste is advisory and must not become Engineering Fact.
- Model-specific tuning notes: model routing remains owned by the runtime configuration.
- The 21 craft standards: broad product/design/writing policy is outside this pipeline's scope.

## Provenance and exit

Source: `https://github.com/apoorvjain25/frontier`.
Pinned revision: `0b39eee3ec9ed0905ad7303772653c7e9bd17831`.
License: MIT; retain source and license attribution when copied text or substantial portions are
distributed. This document restates method semantics in Pipeline-owned language.

Remove this reference if the local rules become independently specified and tested, if the source
license or provenance changes, or if the method adds review cost without measurable defect capture.
