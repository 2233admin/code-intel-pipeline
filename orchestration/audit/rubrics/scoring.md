# Scoring Rubric

Adapted from [Fuck_My_Shit_Mountain](https://github.com/XiNian-dada/Fuck_My_Shit_Mountain) (MIT).

## Principle

Scores are judgment-based, not formula-based. A department scores its dimension holistically from the findings and coverage it actually produced; there is no mechanical per-severity deduction table. Mechanical deductions produce false precision and inflated scores.

## Scale

Each assessed department scores **0.0 - 10.0, higher is better**.

| Score | Meaning |
|-------|---------|
| 10.0 | Clean for the inspected scope. No findings, and coverage is High. |
| 0.0 | Unacceptable. Pervasive Critical-severity findings. |

**Direction is fixed: 10 is best, 0 is worst. Do not reverse it.**

## The Zero-Findings Rule

A department with zero findings may score `10.0` **only when its coverage row is `High`**. This is not a style guideline — the kernel's `validate()` enforces it as a fail-closed invariant and rejects a report where a `10.0` department has zero findings but coverage below `High`.

Under Medium or Low coverage, a department with zero findings still scores below 10.0 and the score dashboard's justification must say the absence of findings reflects limited coverage, not a clean system.

## Not-Assessed Departments

A department whose `status` is `not_assessed` or `disabled` always scores `score: null`. Its coverage row is always `not_assessed`. Null-scored departments are **excluded** from `score_dashboard.overall` — they are not averaged in as zero, and they do not count toward the denominator.

## Unknown Never Earns a Health Bonus

This repo already treats unmeasured evidence as a liability, not a free pass: `docs/hospital-mode.md` states that unknown measurement evidence does not receive a health bonus, and unknown-status dimensions score `0` rather than being skipped favorably. The audit layer keeps the same fail-closed direction, adapted to a `null`/`not_assessed` overall-score exclusion instead of a `0`, because an audit dimension's `null` genuinely means "not measured this run" rather than "measured and found acceptable." Either way, the rule is the same: **an unmeasured dimension must never look the same as, or better than, a measured clean one.** A missing or `unknown` applicability check, an inaccessible modality, or a skipped department must never be reported as if it had passed.

## Overall Score

`score_dashboard.overall` is the mean of every **non-null** entry in `score_dashboard.entries`, rounded to one decimal place. It is `null` when no department produced a score. The kernel's `validate()` recomputes this mean from the entries and rejects a report where the stated `overall` does not match.

## Score Anchors

Use these as guidance, not fixed rules — the final score is the department's judgment.

| Range | Meaning |
|-------|---------|
| 9.0 - 10.0 | Clean or near-clean. Only minor, isolated findings. |
| 7.0 - 8.9 | Real findings exist but are contained; local fixes, not rewrites. |
| 5.0 - 6.9 | Systemic issues; multiple Medium-or-higher findings; needs deliberate investment. |
| 3.0 - 4.9 | Serious, non-isolated High-severity findings; needs structural change. |
| 0.0 - 2.9 | Pervasive Critical-severity findings; the approach in this dimension is broken. |

## Rules

1. Every non-null score entry needs a one-sentence `justification` naming the strongest evidence and, when coverage is below High, naming the limitation.
2. Do not average finding severities into the score. One systemic Critical finding can outweigh many isolated Low findings, and the reverse.
3. Consider density: many Low-severity findings concentrated in one area can warrant a lower score than a single Critical finding with a trivial fix.
4. Adjust for context — the same finding can mean different things in a CLI tool versus a security-critical library.
5. Round `overall` to one decimal place. Do not round a department's own score to flatter it.
6. A department that is `not_assessed` or `disabled` is reported as such, never silently omitted from `departments`, `score_dashboard.entries`, or `coverage_matrix`.
