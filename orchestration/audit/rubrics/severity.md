# Severity Rubric

Adapted from [Fuck_My_Shit_Mountain](https://github.com/XiNian-dada/Fuck_My_Shit_Mountain) (MIT).

Severity describes the engineering impact of a finding if it is left unaddressed. It is independent of confidence (how sure the department is) and of coverage (how much of the dimension was inspected). Use exactly these five levels: `Critical`, `High`, `Medium`, `Low`, `Info`.

A department assigns severity from what the admitted modality evidence (`xray`, `anatomy`, `ct`, `mri`, `pet`, `chart`, `governance`) and any targeted follow-up reads actually show — not from how alarming the finding sounds.

## Critical

- Remote code execution, arbitrary command execution, or privilege escalation reachable from an untrusted input.
- Secret, credential, or token material committed to source, written into an artifact, or echoed into logs in plaintext.
- A dependency with a known exploited vulnerability on a code path the repo actually exercises.
- Data loss or corruption on a normal, non-adversarial run.
- A fail-closed guard (an admission check, a gate, a hospital discharge rule) that can be bypassed, silently disabled, or that fails open instead of closed.
- A production-facing entry point with no authentication or authorization check.

## High

- Authorization bypass or privilege escalation confined to the application's own boundary.
- Injection (command, SQL, path traversal) with a realistic, evidence-backed attack surface.
- An unhandled panic or crash on input the modality evidence shows is expected or already occurs in practice.
- A structural or architecture-graph violation (`ct`/`anatomy` evidence) that breaks an enforced boundary.
- A published artifact or schema changed in a way that breaks an existing consumer without a version bump.
- No test coverage on a path the evidence marks as critical (fail-closed logic, artifact publication, admission checks).

## Medium

- Output or error content that leaks internal paths, identities, or state to a less-trusted consumer.
- A retry, external call, or provider dependency with no timeout or backoff (`pet`/`ct` evidence of the call site).
- Unbounded growth of a collection or artifact under normal, expected load.
- A large, unclear-responsibility module the structural evidence (`ct`) already flags as a hotspot.
- Missing error handling on a non-critical path.
- A flaky or non-deterministic check surfaced by `governance`/CI evidence.
- Duplicated logic across modules that the evidence shows diverging over time.

## Low

- Style or formatting issues that do not affect correctness.
- Missing explanation on logic the evidence shows is genuinely non-obvious.
- An untested edge case on a path the evidence marks as low-risk.
- Dead code identified by evidence (no incoming references) that is not actively harmful.
- A documentation statement the evidence shows is out of date.

## Info

- An observation that is not itself a risk but is useful context for a later run.
- A pattern that could become a risk at a different scale, noted for the record.
- A candidate finding that does not meet the evidence bar for any severity level above — report it as Info rather than discarding it, since fail-closed departments must not silently drop what they saw.
