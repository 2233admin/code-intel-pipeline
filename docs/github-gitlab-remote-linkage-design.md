# Design: GitHub/GitLab/self-hosted-git remote linkage for repowise-indexed repos

- Status: proposed (research + design only, no implementation)
- Date: 2026-08-05
- Scope: proxy-layer enrichment only. No repowise source or binary is modified.

## 1. Problem

repowise indexes ~135 local repos in the `D:\projects` workspace but has zero concept of a
remote git host (confirmed via `repowise --help` / `repowise workspace add --help`: no
GitHub/GitLab flags, no remote-API integration anywhere in its command surface). Its analysis
is 100% local-filesystem + local-git-log based (blame, commits, co-change). Users viewing the
repowise web UI or generated reports currently have no way to jump from a repo/file/commit to
its GitHub/GitLab/Gitea web page.

This document designs a **code-intel-owned enrichment layer**, sitting entirely in the existing
HTTP reverse-proxy (`repowise_proxy_server.rs` / `repowise_i18n_proxy.rs`), that resolves each
indexed repo's remote and injects deep-link URLs into what the UI renders — without touching
repowise's source, binary, or database.

## 2. What repowise already has (verified against live data)

### 2.1 Per-repo storage is not centralized

Every repo repowise indexes gets its **own** `.repowise/wiki.db` inside that repo's directory
(e.g. `D:\projects\code-intel-pipeline\.repowise\wiki.db`, 19.3 MB). The workspace root
(`D:\projects`) also has its own `.repowise\wiki.db` (1018 MB) and treats itself as a
pseudo-repo named `projects`. A filesystem walk of `D:\projects` (depth-bounded, 6 levels)
found **136** `.repowise` directories — one per workspace member. There is no single
"workspace registry" file we could find (`repowise workspace list` / `scan` presumably
discover members by walking for `.repowise` dirs or via internal state we didn't need to
reverse-engineer, since we're not touching repowise's internals regardless).

### 2.2 The `repositories` table already has a `url` column — but it's dead

Schema (`PRAGMA table_info(repositories)`) on both the workspace-root db and a sampled
member db (code-intel-pipeline's own):

```
id             VARCHAR(32)  PK   -- looks like a random uuid4().hex, NOT derived from local_path
                                  -- (verified: md5/sha1/sha256 of local_path in 4 case/slash
                                  --  variants does not match the stored id)
name           VARCHAR(255)
url            TEXT         --  <-- exists, but...
local_path     TEXT
default_branch VARCHAR(255)
head_commit    VARCHAR(40)
settings_json  TEXT
created_at / updated_at DATETIME
total_commit_count, first_commit_at, total_contributor_count, ...
```

We queried `url` across **all 136** indexed repos (workspace root + every member db), not just
a sample: **`url` is an empty string in 136/136 rows**, including `code-intel-pipeline` itself,
which has an active `origin` remote at `github.com`. repowise never populates this column from
`git remote get-url origin`. It's present in the schema (possibly a vestige of an earlier or
planned "clone from URL" workflow) but is not a usable data source. **Conclusion: code-intel
must own this data itself; there is nothing to piggyback on.**

No table named anything like `remotes`, `hosts`, or `origins` exists among the 42 tables in
`wiki.db`. `external_systems` (183 rows) is about dependency ecosystems (npm/pip/cargo
packages), not git remotes — a false lead, ruled out.

### 2.3 The `id` primary key's stability is not proven, so don't rely on it alone

`id` is a 32-hex-char value that isn't a deterministic function of `local_path` (tested and
ruled out). It's consistent with `created_at`/`updated_at` being separate columns (i.e. a
persisted row updated in place by `repowise update`, not recreated), which suggests `id` is
stable across normal incremental reindexing — but this is an inference from schema shape, not
something we verified by actually running `repowise update` and diffing the id (we didn't do
that, to avoid mutating index state as a side effect of a research task; flagged as an open
question in §8). It is **not** safe to assume `id` survives `repowise delete` + `repowise init`,
or a fresh clone/index on another machine. See §4.1 for the keying decision this drives.

## 3. Real remote-URL corpus from this environment (not hypothetical)

Script: enumerate all 136 `.repowise` dirs found under `D:\projects` (depth ≤ 5), for each
check `.git` presence and run `git -C <path> remote get-url origin`. Full corpus, not a
sample:

| Category | Count | Notes |
|---|---:|---|
| `github.com` | 97 | e.g. `code-intel-pipeline`, `browser-bridge`, `WorldMonitor` (owner `2233admin`); `autoresearch` (owner `smallnest`, a fork/upstream); `OpenAlice-latest` (owner `TraderAlice`, remote name `OpenAlice` — repo dir name and repo name diverge) |
| self-hosted Gitea, `git.xart.top:8418` | 18 | e.g. `k-atana`, `tdxcli-rs`, `workbench-kernel` (remote repo is named `katana-kernel`, workspace alias is `workbench-kernel` — another name/alias divergence) |
| local git repo, **no `origin` remote configured** | 21 | e.g. `DaHuaCyou_run` — `git remote get-url origin` fails with "No such remote 'origin'" |
| no `.git` at all (pure local snapshot) | 0 observed here | Not present in this corpus, but repowise's docs and `workspace add` allow indexing a bare directory — the design must still degrode gracefully for this case |

Sub-findings inside the Gitea group, all real and all load-bearing for the design:

- **2 of 18** use plain `http://` instead of `https://` (e.g. a couple of the `_archive/2026-04-batch2/*` repos) — mixed-scheme, must be preserved/normalized rather than assumed.
- **1 of 18** (`red-queen`) has no `.git` suffix on the remote URL, unlike the rest — URL
  normalization must not assume the suffix is present.
- **3 of 18** have **plaintext credentials embedded in the remote URL**
  (`https://<user>:<token>@git.xart.top:8418/...`, a real personal-access-token). **This is a
  live secret in this machine's `.git/config` files, not a hypothetical.** It is called out in
  detail in §7.1 because it drives a hard requirement on the design, but the actual token value
  is intentionally not reproduced anywhere in this document — treat any code that reads remote
  URLs as handling potentially credentialed strings.

No `gitlab.com`, self-hosted GitLab, or GitHub Enterprise instance exists in this environment's
sample. The design below still needs to handle them (via the override mechanism in §5), but
that support is **unverified against a live instance in this environment** — flagged honestly
as such rather than presented as tested.

## 4. Data model

### 4.1 Where it lives: a code-intel-owned sidecar, never inside `wiki.db`

Per the task's own framing (and confirmed by watching how repowise treats its own db — no
"external metadata" extension point, no plugin table, and the `url` column shows it doesn't
even reliably fill in *its own* schema) — the cache **must** live outside `wiki.db`, so it
survives `repowise` reindexes, `repowise delete`/`init` cycles, and version upgrades cleanly.

Follows the existing `CODE_INTEL_DATA_ROOT` convention already implemented in
`crates/code-intel-cli/src/doctor_bootstrap/paths.rs` (`data_root()`): per-user data dir,
`CODE_INTEL_DATA_ROOT` env override, else platform default (Windows:
`%LOCALAPPDATA%\code-intel\code-intel`, else `~/.code-intel`). Proposed location:

```
<CODE_INTEL_DATA_ROOT>/remote-links/registry.json
```

A single JSON file is enough at this scale (136 repos today; even 10x that is trivial JSON).
No new SQLite dependency needed — `code-intel-cli` doesn't currently link `rusqlite`/similar
for anything outside talking to repowise's db path constants, and pulling in a DB dependency
for a <1MB cache would be disproportionate.

### 4.2 Keying: `local_path`, not repowise's internal `id`

Decision, and why: repowise's workspace/status JSON responses (the ones the proxy already
intercepts) key everything by `local_path` (that's literally what `repowise workspace list`
displays and what's in every `repositories` row) — so the proxy can join cache entries onto
proxied responses using a field that's *already there*, with no dependency on repowise's
internal `id` stability. `local_path` also degrades correctly for the "no git", "git but no
remote" cases (§3) — the cache still needs an entry for those, just with `remote: null`.

Trade-off accepted: `local_path` breaks if a repo directory is moved or the cache is copied to
a different machine. That's fine — a cache miss is cheap to repair (one `git remote get-url`
shell-out), unlike trying to keep a cross-machine-portable identity in sync. Normalize
Windows-vs-forward-slash and case before using it as a key (Windows paths are case-insensitive;
we saw this matters — `local_path` values come back exactly as repowise stored them, e.g.
`D:\\projects\\code-intel-pipeline`, and any join key derived by us independently, e.g. from a
`git -C` invocation, must canonicalize to match).

`repositories.id` (repowise's own row id) is recorded as an **opportunistic secondary index**
only, when it appears in a proxied response, to make lookups O(1) instead of a path scan — but
it is never treated as the durable identity, per §2.3.

### 4.3 Cache entry shape (illustrative, not final wire format)

```json
{
  "D:\\projects\\code-intel-pipeline": {
    "remote_url_normalized": "https://github.com/2233admin/code-intel-pipeline",
    "host_type": "github",
    "host": "github.com",
    "owner": "2233admin",
    "repo": "code-intel-pipeline",
    "web_base_url": "https://github.com/2233admin/code-intel-pipeline",
    "has_credentials_stripped": false,
    "checked_at": "2026-08-05T00:00:00Z",
    "git_head_at_check": "ee975457d501cd336ed80536791d061633617e6c"
  },
  "D:\\projects\\DaHuaCyou_run": {
    "remote_url_normalized": null,
    "host_type": "none",
    "checked_at": "2026-08-05T00:00:00Z"
  }
}
```

`remote_url_normalized` is **always credential-stripped and `.git`-suffix-normalized** before
it is written to disk — see §7.1. `has_credentials_stripped` records whether stripping actually
fired, purely as an internal signal (e.g. to warn the operator once that a token was found in a
`.git/config`, without ever writing the token itself anywhere, including logs).

## 5. Host detection + override

### 5.1 Detection from the URL alone (covers the majority case)

Regex/string match on the host component of a normalized remote URL:

- `github.com` → `github`
- `gitlab.com` → `gitlab`
- anything else → `generic` (unknown host, can still show the raw link, cannot deep-link to
  file/line — see 5.3)

This alone resolves 97/115 (97 github.com out of 115 repos-with-a-remote) in this environment
— the overwhelming majority. But it does **not** resolve the 18 `git.xart.top:8418` repos: a
custom hostname on a custom port gives no signal about which product is running behind it. We
confirmed this isn't solvable by smarter regex — `git.xart.top` doesn't contain "gitea",
"gitlab", or any other identifying token. This is exactly the case ADR-adjacent tooling in this
workspace already anticipated: `docs/adr/0001-merge-mcm-rust-unified.md` names a planned
`sync` crate with three explicit backends — `gitea / github / gitlab` — for a *different*
concern (my-code-machine's host-machine checkpoint sync), but it's the same three-way
enumeration this design needs, and confirms self-hosted-vs-which-product is a known,
recurring problem in this monorepo, not a novel one. (That `sync` crate does not exist yet —
only `code-intel-cli` and `code-nexus-lite` crates exist in `crates/` today — so there's
nothing to import from it now, but if it's ever built, host-type detection is a natural shared
utility to de-duplicate into.)

### 5.2 Override for anything that can't be auto-detected

Env var: `CODE_INTEL_GIT_HOST_OVERRIDES`.

**Shape decision**: a *file path*, not inline JSON in the env var value — deliberately
diverging from the `model_channels.rs` CC-Switch shape (`CODE_INTEL_CC_SWITCH_ENDPOINT` /
`_API_KEY`, which fetches from a *live HTTP endpoint*). The override list is static local
config, structurally identical to the existing `CODE_INTEL_INTEGRATIONS_MANIFEST` pattern in
`capability.rs::discover_manifest()` (env var → file path → `.is_file()` check → fallback
candidate paths under `CODE_INTEL_HOME`/`CODE_INTEL_DATA_ROOT`). Inline JSON in a Windows
PowerShell env var is exactly the quoting pain ADR 0001 names as its #1 motivating complaint
("every time PowerShell trips me up it kills efficiency") — a file path sidesteps that
entirely, so the manifest shape is the better-fitting precedent here, even though CC-Switch is
the more recently-added example.

Default discovery order (mirrors `discover_manifest`): explicit CLI flag (if one is ever added)
→ `CODE_INTEL_GIT_HOST_OVERRIDES` (file path) → `<CODE_INTEL_HOME>/orchestration/git-hosts.json`
→ absent means "no overrides, generic-git fallback only."

Manifest shape:

```json
{
  "git.xart.top:8418": {
    "type": "gitea",
    "web_base_url": "https://git.xart.top:8418"
  }
}
```

Keyed by `host:port` (not just host) since we observed a real instance running on a non-default
port. `type` is one of `github | gitlab | gitea | generic`. `web_base_url` lets the override
correct scheme/port if the git remote scheme and the web UI scheme ever diverge (uncommon but
not guaranteed to match).

### 5.3 What the override actually buys you (the part that must be precise)

Without the override, `git.xart.top:8418` repos fall into the `generic` bucket, and the design
can still show *something*: the normalized, credential-stripped remote URL as a bare link to
the repo root. What it **cannot** do without knowing the specific product is generate correct
**file/line deep links**, because the three products use structurally different path schemes:

- GitHub: `/{owner}/{repo}/blob/{branch}/{path}#L{line}`
- GitLab: `/{owner}/{repo}/-/blob/{branch}/{path}#L{line}`
- Gitea: `/{owner}/{repo}/src/branch/{branch}/{path}#L{line}`

These are public, documented URL conventions for each product (not verified against the live
`git.xart.top` instance in this session — deliberately not probed, since 3 of the 18 Gitea
remotes carry embedded credentials and poking at that host wasn't in scope for a research-only
task). So: **the override's value-add is specifically per-file/per-line deep-linking**; host
identification by itself only changes which fallback label is shown next to a bare link. State
this precisely in any UI copy — "generic" should read as "linked, but only to the repo root,"
not "unlinked."

## 5.4 Standing-decisions crosswalk (explicit, per all three named ADRs)

The task named three standing decisions to check this design against. All three were read in
full (`docs/adr/0008-local-first-project-control-plane.md`,
`docs/adr/0009-atomic-capability-execution-model.md`,
`docs/adr/0001-merge-mcm-rust-unified.md`). Verdict for each:

- **ADR 0008 (Select the Project Control Plane Locally First) — no conflict.** ADR 0008 governs
  *mutable task/PM state authority* ("maintain exactly one writable source of mutable task state
  per initiative"). The remote-link sidecar (§4) is derived, recomputable cache data about git
  remotes — never task intent, ownership, or review state. It doesn't compete with, mirror, or
  need to be a "writable source of task state." Nothing in this design touches Linear, a Work-OS,
  or any task graph. Orthogonal, not merely non-conflicting.
- **ADR 0009 (Adopt an Atomic Capability Execution Model) — no conflict, by existing precedent.**
  ADR 0009's envelope/Snapshot-Identity/Artifact-Ref/Effect-Boundary contract governs things
  "moving across the orchestration boundary" — i.e. capability atoms invoked through
  `cli::run()`. The proxy layer this design extends is **not** in that boundary today: both
  `repowise_proxy_server.rs` and `repowise_i18n_proxy.rs` are reached via a raw pre-dispatch
  branch in `main.rs` (`if raw[0] == "repowise-proxy"`) before `cli::run(&raw)` is ever called,
  and neither uses capability request/result envelopes. This design's new module
  (`git_remote_registry.rs`, §9) follows that same, already-established precedent rather than
  introducing a new exception — it is UI-serving glue, not a capability atom, and ADR 0009 does
  not currently require UI-serving glue to carry envelopes. Flagged as worth one line of
  reviewer attention only if code-intel later decides *all* proxy-layer plugins should be
  formalized as capability atoms — that would be a change to existing practice (affecting the
  i18n proxy too), not something this design introduces unilaterally.
- **ADR 0001 (Merge my-code-machine into code-intel-pipeline, Rust-unified monorepo) — fits,**
  detailed in §5.1 and §9: the gitea/github/gitlab three-way enumeration ADR 0001 names for its
  (not-yet-built) `sync` crate is the same enumeration this design needs, for a different
  purpose (web deep-links vs. host-machine checkpoint sync). No conflict; flagged as a future
  de-duplication opportunity, not a blocker, since that crate doesn't exist yet.

## 6. Where detection/generation runs, and where the proxy surfaces it

### 6.1 Precompute + cache, not on-demand-per-request

`git remote get-url origin` is a subprocess spawn per repo. At 135+ repos, doing this inline on
every proxied HTTP request (the workspace list page alone returns all of them) is wasteful and
adds latency to a hot path. Precompute at a natural checkpoint instead:

- On proxy startup (`repowise_proxy_server::start_proxy`), warm the cache once in a background
  thread (the proxy already spawns a thread per request via `thread::spawn`, so a startup
  warm-up thread is a small, consistent addition to that same file's shape) — not blocking the
  first request.
- Refresh lazily: on a cache miss (new repo added to the workspace since last warm-up) or when
  a proxied response's `local_path` isn't in the registry, resolve on that request and persist
  the result, rather than re-scanning all 135+ repos on every miss.
- No polling/watch loop — `git remote get-url origin` for a given repo essentially never
  changes at request-serving timescales; a manual "resync" trigger (see §8) is enough.

### 6.2 Surface it as a small proxy-served JSON endpoint, not baked into every payload

`repowise_i18n_proxy`'s pattern for the client-rendered UI is to inject a `<script>` with a
`MutationObserver` because server-side string replace can't reach React-rendered text. The
same constraint applies here — repo names/links render client-side. But unlike the i18n
dictionary (a few dozen static string pairs, cheap to inline into every HTML response), the
remote-link table is proportional to workspace size and only needed by JS running in the
browser, not by every proxied response. Recommended shape:

- New proxy route, e.g. `GET /__code-intel/remote-links.json`, served directly by
  `repowise_proxy_server` (short-circuited before the upstream forward, the way a reverse proxy
  typically owns synthetic routes) — returns `{ "<local_path>": { "web_base_url": ..., "host_type": ... }, ... }` from the sidecar cache.
- The injected script (extending the same `build_injection_script` mechanism already in
  `repowise_i18n_proxy.rs`, or a small sibling script block) `fetch()`s that endpoint once,
  builds a `local_path → link` map in memory, and the same `MutationObserver`-driven pass that
  already walks translated text nodes also rewrites repo-name elements (which presumably carry
  the path in a `data-*` attribute or are joinable via the same JSON the workspace API already
  returns to the page) into anchor tags.
- This keeps `repowise_i18n_proxy.rs` and `repowise_proxy_server.rs` themselves untouched by
  this design doc's proposal in the sense of not needing new logic *inside* those exact
  functions read for this task — the new capability is a new sibling module + a new route
  registered alongside the existing forwarding logic, following the same file-per-plugin shape
  those two files already establish. (The task's instruction not to modify those two files is
  honored at the design level; actual wiring will necessarily touch `repowise_proxy_server.rs`'s
  routing dispatch when implemented, since that's the only place requests are routed at all —
  flagged plainly here rather than glossed over.)

### 6.3 Integration point is proxy-layer only — confirmed, with one explicitly excluded alternative

Checked whether anything besides the HTTP-proxied Next.js UI consumes repowise repo metadata in
a way that would want the same enrichment:

- **MCP tool surface** (`mcp__repowise__*`, loaded in this very session): `providers.rs` in
  `code-intel-cli` documents repowise's "MCP/HTTP serve surfaces for agent callers" as a
  provider capability, but there is no code-intel-owned wrapper around repowise's MCP server —
  agent callers (including this session) talk to repowise's MCP process directly, stdio, with
  no interception layer today. Enriching MCP tool responses (e.g. `get_overview`,
  `get_context`) with remote links would require a *new* stdio proxy analogous to the HTTP one
  — a real, separate piece of infrastructure, not a reuse of `repowise_proxy_server.rs`.
  **Excluded from this design's scope explicitly** (not silently dropped) — noted as a natural
  Phase-later extension in §9 if agent-facing deep links become a stated need.
  - Note this is scope of *repowise's* MCP surface being unproxied — it's a different question
    from whether *code-intel's own* future tools should include remote links; if code-intel
    ever exposes its own MCP tools that describe a repo, they can read the same sidecar
    registry directly (no proxy needed, since they're code-intel's own process).
- `repowise export` / `repowise generate-claude-md`: write files directly from the repowise CLI
  process, not through the HTTP proxy at all. Also excluded from this design's scope for the
  same reason as MCP — a different, non-HTTP wrapper would be needed. Worth flagging since the
  task description says "surfaced in the UI/**reports**" — if "reports" means generated
  markdown/CLAUDE.md content rather than just the web UI, that's additional scope this document
  does not cover and should be scoped separately.

Net: **for the web UI specifically, proxy-layer-only is correct and sufficient** — confirmed,
not assumed. For "reports" more broadly, flagged as an open question in §8 rather than silently
narrowed.

## 7. Risks

### 7.1 Credential handling (hard requirement, not a nice-to-have)

3 of the 136 indexed repos' `origin` remotes contain plaintext `user:token@host` credentials
(a real Gitea personal access token, observed directly in this environment — not reproduced in
this document). Any code that runs `git remote get-url origin` **will** see these strings.
Hard requirements this drives:

1. Strip userinfo (`user[:password]@`) from the remote URL **before** it is normalized, cached,
   logged, or ever placed in an HTTP response body — no code path may persist or transmit the
   raw remote URL.
2. The sidecar cache file (§4) must be added to `.gitignore` if it could ever land inside a
   repo directory (it won't, per §4.1 — it's under `CODE_INTEL_DATA_ROOT`, outside any repo —
   but this must stay true; don't let a later refactor move it under a repo root "for
   convenience").
3. `repowise_proxy_server`'s existing logging (`eprintln!` for upstream errors) must not log
   raw remote URLs either, if any error path ends up including one.
4. 2 of 18 Gitea remotes use plain `http://` — not a code-intel bug to fix, but worth a
   passive `is_plaintext_transport: true` flag in the cache entry so a future health/security
   check (`get_health`/`get_risk`-style surfacing this codebase already has conventions for)
   could flag it, without this design doc scope-creeping into fixing it.

### 7.2 Name/alias divergence

Two real cases found: `OpenAlice-latest` (workspace dir name) → remote repo `OpenAlice`;
`workbench-kernel` (workspace alias) → remote repo `katana-kernel`. Confirms §4.2's decision to
key by `local_path`, never by repo display name — display name is cosmetic and repowise-chosen,
not a reliable join key to the remote.

### 7.3 Windows path normalization

`local_path` values from repowise come back as `D:\\projects\\...` (backslash, drive letter).
Any independently-derived path (e.g. from `std::env::current_dir()` or `git -C` output) must be
canonicalized to the same casing/separator convention before being used as a cache key, or
lookups will silently miss. This is a real, previously-seen failure class in this project (see
project memory: `CODE_INTEL_HOME worktree trap`, `code-intel primary-entry env traps` — both
about environment/path mismatches across worktrees) — worth explicit test coverage, not just a
mention.

### 7.4 Cache staleness across `repowise` reindex/upgrade

Addressed structurally by §4.1 (cache lives outside `wiki.db`), but a repo whose remote changes
(e.g. `git remote set-url origin ...`) won't be reflected until the next lazy-refresh trigger.
Acceptable for a first version; a manual `code-intel ... --resync-remotes`-style trigger is
cheap to add later (see §9) if staleness becomes an actual complaint rather than a theoretical
one.

## 8. Open questions

1. **Does repowise's `repositories.id` survive `repowise update`?** Inferred likely-yes from
   schema shape (separate `created_at`/`updated_at` implies row-level upsert), not empirically
   verified — deliberately not tested by mutating index state as a side effect of this
   research task. If someone verifies this before implementation, `id` could become a valid
   fast-path secondary index with more confidence than §4.2 currently grants it.
2. **Does "reports" in the task's framing mean more than the web UI?** (§6.3) If generated
   markdown/CLAUDE.md output should also carry remote links, that's a second, non-HTTP
   integration surface this document intentionally scoped out — needs an explicit decision,
   not a default assumption either way.
3. **Should `git.xart.top:8418`'s actual URL scheme be verified against a live fetch before
   shipping the Gitea path template in §5.3?** Deliberately not done in this session (avoided
   touching a host that 3 of its remotes reach with embedded credentials). A safe verification
   would use one of the 15 *non*-credentialed `git.xart.top` remotes and a read-only page fetch,
   done explicitly and separately from this research pass.
4. **Multi-remote repos**: this design assumes `origin` is the relevant remote (matches the
   task's own instruction to check `git remote get-url origin`). Not checked: whether any of
   the 136 repos have a second remote (e.g. `upstream`) that would be the "real" public-facing
   one for linking purposes — `autoresearch` (owner `smallnest`, likely a fork) is a candidate
   worth spot-checking for a second remote before assuming `origin` is always right.

## 9. Phased implementation plan (planning only — nothing below is implemented)

**Phase 0 — confirm, don't build.** Resolve open questions #1 and #4 above (cheap, read-only
checks) before writing any code, since both could change the keying/remote-selection decision.

**Phase 1 — sidecar store + host detection (no proxy wiring yet).** New module,
`crates/code-intel-cli/src/git_remote_registry.rs` (flat sibling to `repowise_i18n_proxy.rs`
and `model_channels.rs`, matching this crate's current structure — no `sync`/`machine` crate
exists yet per ADR 0001, so this stays inside `code-intel-cli` for now, with a note that it's a
candidate to move into a shared crate if/when ADR 0001's `sync` crate is ever built, since that
crate is slated to need the same gitea/github/gitlab enumeration for a different purpose).
Implements: URL normalization + credential stripping (§7.1), host detection (§5.1),
override-manifest loading (§5.2, mirroring `capability::discover_manifest`), the JSON sidecar
read/write (§4). Unit-testable in isolation against the real corpus shapes found in §3 (github
with/without `.git` suffix, Gitea with custom port, Gitea with embedded credentials, no-origin,
no-git) as golden-file-style test fixtures — this project's existing test conventions already
favor that (see `crates/code-intel-cli/tests/capability_exec.rs`).

**Phase 2 — cache warm-up + lazy refresh.** Wire the background warm-up into
`repowise_proxy_server::start_proxy` (a startup thread, per §6.1), plus the on-miss lazy-refresh
path. No user-facing surface yet — verify via direct cache-file inspection that it populates
correctly against the live 136-repo workspace before adding any HTTP surface.

**Phase 3 — proxy route + client-side injection.** Add the `/__code-intel/remote-links.json`
route to `repowise_proxy_server`'s request dispatch (§6.2), and the client-side fetch +
DOM-annotation script, following `repowise_i18n_proxy::build_injection_script`'s existing
pattern (content-based dedup, `requestAnimationFrame`-batched mutation handling — both already
solved problems in that file, reuse the approach rather than re-deriving it).

**Phase 4 — security review pass specific to §7.1.** Given the live-credential finding, this
phase is not optional/skippable: explicit test coverage proving no code path (cache file,
`/__code-intel/remote-links.json` response, any log line) ever contains an unstripped
credential, run against the actual 3 credentialed remotes in this environment as a regression
fixture (with the token itself excluded from the fixture/test file — assert only that stripping
happened, don't hardcode the real token as expected input in a committed test).

**Phase 5 (optional, only if §8 Q2 resolves "yes") — extend beyond the HTTP proxy.** Only if
"reports" is confirmed to mean more than the web UI: a separate, smaller design for wrapping
`repowise export`/`generate-claude-md` output, out of scope for this document.
