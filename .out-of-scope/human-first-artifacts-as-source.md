# Non-goal: human markdown artifacts as the source of truth

## Non-goal

`summary.md`, `understanding.md`, and other human-facing Markdown views of an artifact run are not
a goal to make authoritative. Code Intel Pipeline will not grow features, tests, or downstream
tooling that read these files back in as data, hand-edit them, or treat their absence/presence as
a pass/fail signal.

## Why

CONTEXT.md already draws this boundary for the domain: a **Materialized View** is a "Rebuildable
human or index projection derived from machine artifacts, such as summary Markdown or the
cross-repository index," and explicitly lists what to avoid — "Source of truth, artifact producer,
mutable task state." The machine artifacts (`report.json`, `run-complete.json`, Artifact Refs with
their SHA-256 identity and Snapshot Identity) are what Run Commit publishes as authoritative;
Markdown is a rebuildable rendering of that data for a human reader, produced after the fact. If a
Markdown view became a thing other code depended on, every future formatting or wording change
would silently become a breaking change to an implicit contract nobody wrote down. Refs #101.

## Instead

- Compute and gate everything from the machine artifacts (JSON, schemas, Artifact Refs). Markdown
  is generated last, from already-validated data, and only for a human to read.
- If a workflow needs to check something a report says, read the JSON field, not the rendered
  Markdown line.
- If Markdown drifts from the artifacts it was built from, regenerate the Markdown — never hand-fix
  the Markdown and leave the artifacts as they were.
