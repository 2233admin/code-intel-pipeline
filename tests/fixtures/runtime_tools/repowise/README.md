# Real Repowise runtime samples

The recorded files are actual pre-migration public CLI captures using the unchanged c6fefea consumer and official Repowise 0.38.0, before any product edit in #419. Source fixture: one Python file declaring and calling `greet`. The candidate 0.55.0 does not supply approved expectations.

Request JSON contains the original public provider invocation with the temporary repository replaced by `$SANDBOX`. Replay runs status on an empty repository, init through index, status on its generated index, then index on the uncommitted repository (the real missing-sync fallback). It commits only its owned source and indexes that commit, then explicitly adds a `farewell` function to the fixture and commits that input change before the final index request. The final request therefore exercises an incremental changed-source update, not another missing-sync initialization or unchanged-source no-op. Provider source immutability is checked against the exact fixture bytes supplied to each invocation. Raw captures retain the complete public CLI response, including its upstream `stdoutTail` diagnostics and acquisition provenance; they are not overwritten after an upgrade.

The compared response is the Pipeline-owned diagnostic/authority envelope: schema, provider, operation, artifact, classification, evidence, factPromotionEligible, exitCode, ok, stderrTail. The artifact path comes from the original recorded 0.38.0 replies; only its temporary absolute repository root and platform separators are normalized, preserving the exact relative `.repowise/wiki.db` path and rejecting paths outside the fixture. Successful indexing must also produce its declared artifact and leave input source bytes unchanged. Upstream `stdoutTail` is a raw provider presentation surface, not an admitted Engineering Fact: its changing console layout stays in recorded diagnostics, not in the Pipeline envelope approval. No error classification, failed exit or authority flag is scrubbed.

The additional `--previous-provider-bin` replay creates the index with the actual recorded-version SDK, commits the owned source, then switches executables over that same index for status and the incremental changed-source update with the candidate SDK. It neither fabricates database contents nor imports provider storage internals. CI requires both exact acquisition versions and also runs the candidate-only fresh-index replay; the old SDK is an isolated compatibility baseline, not a live production target.

The isolation-excluded Windows workstation acquires binary wheels only. CI keeps the same rule except for the official `watchdog<5` dependency on macOS: those wheels predate Python 3.14, so only that non-excluded runner builds its upstream source distribution. The provider and Python targets, actual version checks, real replay, and failure gates are unchanged.

Temporary absolute repository roots differ across runs. Provider version is retained as acquisition provenance rather than asserted equal across the intended upgrade. No timestamps, relative paths, source hashes, business results or error categories are removed from the compared envelope.

Smoke execution uses isolated HOME and credential-free environment plus the documented `DO_NOT_TRACK=1` fixture privacy control. The first candidate smoke passed four operations but failed temporary-directory cleanup because upstream's detached telemetry flusher retained its working directory; that failure is preserved in #419. The control prevents that unrelated external telemetry process, rather than ignoring cleanup errors or changing production provider invocation. This is not an offline claim: index-only can still load remote grammar assets upstream.

Public replay:

```text
code-intel provider --action Invoke --provider repowise --operation status --repo <sandbox-repo> --json
code-intel provider --action Invoke --provider repowise --operation index --repo <sandbox-repo> --json
```

The isolated replay driver exercises those compiled CLI requests:

```text
python tests/test_runtime_repowise.py --cli <compiled-code-intel> --provider-bin <isolated-real-repowise>
```

```text
python tests/test_runtime_repowise.py --cli <compiled-code-intel> --provider-bin <isolated-candidate-repowise> --previous-provider-bin <isolated-recorded-version-repowise>
```
