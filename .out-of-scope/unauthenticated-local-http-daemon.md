# Non-goal: an unauthenticated local JSON-RPC/HTTP daemon

## Non-goal

Code Intel Pipeline will not ship a long-running local HTTP or JSON-RPC server/daemon that
listens on a port and accepts requests without authentication, as a way for agents or tools to
reach pipeline capabilities.

## Why

The pipeline already has a governed agent-facing surface (MCP: "Tool/function definitions exposed
to a model — MCP servers, function-calling schemas, agent tool registries" per
`orchestration/audit/prompts/ai-safety.md`) plus an in-process Effect Boundary and Execution
Policy (`crates/code-intel-cli/src/execution_policy.rs`, `sentrux_gate.rs`) that gate what a
capability may do (repo read, local write, network, repository mutation) before it runs. A bare
local port with no auth is reachable by any process on the machine, sidesteps that boundary
entirely, and creates a second, unaudited agent-facing surface next to the one the ai-safety
review already covers. That is a bigger local attack surface for no capability gain: every request
path a daemon would serve is already reachable through the CLI/MCP surface with policy and
provenance intact.

## Instead

- Expose capabilities as CLI subcommands and MCP tool definitions, both routed through the
  existing Effect Boundary / Execution Policy checks.
- If a long-lived process is genuinely required, require an explicit local auth token or
  Unix-socket/named-pipe scoping, and register it in `orchestration/integrations.json` like any
  other capability — never a bare open port as a default posture.
