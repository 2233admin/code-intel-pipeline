# Model Request Synthesis And Executable Handles

`New-ModelAdapterRequest.ps1` converts a ready routing result and the matching inventory candidate into a closed v2 adapter request. Cost scope, model identity, external-egress posture, and consent states are derived from those inputs; a caller cannot weaken them in a hand-authored synthesis request.

CLI requests use `New-ModelExecutableHandle.ps1`. The short-lived handle binds adapter identity, canonical path, SHA-256, file length, last-write ticks, observation time, and expiry. `Invoke-ModelChannelDelegate.ps1` revalidates every field immediately before starting the process and rejects expired, mutated, moved, or adapter-mismatched executables before invocation.

The handle is content-bound and tamper-evident, not a cryptographic authorization signature: a local actor able to rewrite both the executable and handle can recompute its digest. OS replacement between final verification and process creation also remains a platform-level TOCTOU limit. The control prevents stale or accidentally substituted executables; it does not replace host integrity or code-signing policy.

Raw-path v1 CLI requests are disabled by default. A direct delegate caller must explicitly pass `-AllowLegacyRawExecutable` to enter that compatibility surface; the production `run-code-intel.ps1` facade does not expose or pass that switch. Automatically synthesized requests always use v2 and require a verified handle.
