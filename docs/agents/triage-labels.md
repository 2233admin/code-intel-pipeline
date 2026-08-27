# Triage Labels

These labels define the project-management state machine agents can use when converting Code Intel evidence into Linear work.

| Matt Pocock role | Code Intel / Linear label | Meaning |
| --- | --- | --- |
| `needs-triage` | `needs-evaluation` | Maintainer or agent must decide whether the report is actionable. |
| `needs-info` | `needs-reporter-response` | Waiting on reporter, owner, or operator for missing context. |
| `ready-for-agent` | `ready-for-afk-agent` | Fully specified and safe for an agent to pick up without more human context. |
| `ready-for-human` | `ready-for-human` | Requires human implementation, approval, credentials, or product judgment. |
| `wontfix` | `wontfix` | Will not be actioned. Preserve the reason in the issue or wiki note. |

Use the Code Intel / Linear label as the canonical project label. Keep the Matt Pocock role visible so skills that expect the original vocabulary can map cleanly.

Do not treat a label as proof of scanner evidence. Before moving work to `ready-for-afk-agent`, link the artifact run, relevant report, verification command, and stop condition.
