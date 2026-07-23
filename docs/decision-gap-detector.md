# Decision-gap detector

The C05 detector separates engineering work that can continue deterministically from choices that require explicit authority. It is a classifier, not an interview system and not a decision maker.

## Boundary

A blocker becomes a decision gap only when all discoverable facts named by the blocker have been checked and resolved, no missing fact remains, and the remaining blocker is one of the closed choice kinds in `orchestration/decision-gap-rules.v1.json`. Every emitted gap identifies:

- the blocked decision;
- the discoverable facts already checked;
- at least two options and each option's consequence;
- one recommended answer whose authority kind is `proposal`;
- the exact affected branches.

Missing or unresolved facts are emitted as `factDiscovery` work. They are never converted into questions or decision gaps. Unknown blocker kinds, duplicate branch or blocker identities, and unknown affected branches fail closed.

## Branch-local behavior

Only branches listed in a gap's `affectedBranches` become `blocked_decision_gap`. A branch with missing facts becomes `fact_discovery_required`. Other completed or pending branches retain their state, so deterministic inventory and analysis can continue while a publication choice awaits authority.

The canonical fixture is `tests/fixtures/decision-gap/risk-acceptance.json`: unresolved residual-risk acceptance blocks `publication`, while `inventory` remains `completed`.

## Authority and effects

The detector does not prompt, read interactive input, record answers, emit authority events, adopt proposals, or create committed engineering plans. Its recommendation is explicitly a proposal, its authority state remains unresolved, and its effects list is empty. A separate authority transition must approve any later adoption or commitment.

## Determinism

Results are canonicalized by identifier: branches, gaps, fact-discovery records, facts, options, and affected branch lists have stable ordering. The public module seam is:

```text
load_rule_table(path) -> validated closed rule table
detect(request, rules) -> deterministic detection result
```

Detection is pure after the bounded rule table is loaded and has no network or interactive dependency.
