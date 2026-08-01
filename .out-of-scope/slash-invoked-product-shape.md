# Non-goal: a product shape that requires manual `/skill` invocation

## Non-goal

Code Intel Pipeline will not be designed so that getting value from it depends on a human
remembering to type a slash command (`/code-intel-pipeline`, `/understand`, etc.) at the right
moment. A slash command may exist as one entry point, but it is never load-bearing for whether the
pipeline's evidence reaches an Agent or a human.

## Why

CONTEXT.md defines the **Primary Operator Entry** as "the canonical command surface through which
humans and Agents request pipeline runs... the only entry presented as the default," not "the
thing a human must remember to invoke." Value that only shows up after a manual slash-invocation
fails silently the moment someone forgets, is new to the repo, or is an Agent running unattended —
exactly the audience (see CONTEXT.md's **Agent**, **Agent Loop Pattern**) this pipeline exists to
serve. The pattern this project follows instead is ambient discovery: project instructions
(`AGENTS.md`, CLAUDE.md-style loaders), MCP tool registries, and CI gates surface capabilities and
evidence without a human opting in each time. Requiring a slash command as the only path back to
that evidence would make the pipeline's output as easy to skip as any other optional tool. Refs
#105.

## Instead

- Prefer ambient surfaces an Agent already reads by default: project instruction files, MCP tool
  listings, CI gate output, doctor/bootstrap checks — not a command a human must remember.
- Where a `/skill` entry point exists (see `skills/code-intel-pipeline/`), treat it as one
  convenience wrapper onto the Primary Operator Entry, not the definition of the product.
- If a capability only works when manually slash-invoked, treat that as a gap to close (wire it
  into the ambient surface), not as the shipped design.
