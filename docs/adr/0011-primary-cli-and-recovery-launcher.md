---
status: accepted
date: 2026-07-25
---

# Make code-intel the primary entry and PowerShell the recovery launcher

The compiled `code-intel` command is the only Primary Operator Entry and the
only implementation of Pipeline execution semantics. `archive/code-intel.ps1` is a
cross-platform Recovery Launcher for PowerShell 7.2+: it may locate, validate,
install, repair, update, and start the compiled command, but it must not contain
an alternate Pipeline implementation.

The launcher may automatically restore a missing or invalid installation from
the latest stable Release published by
`github.com/2233admin/code-intel-pipeline`. It must verify the published SHA-256,
stage and switch releases atomically, retain the current and previous verified
stable versions, and prefer the last verified local version when GitHub is
unavailable. A healthy installation is never upgraded during an ordinary run;
updates are explicit.

`code-intel .` and `archive/code-intel.ps1 .` analyze the given Target Repository, or
the current directory when omitted, using `normal` mode by default. The existing
`archive/invoke-code-intel.ps1` remains a quiet forwarding compatibility surface
through v0.x. Agent installation is Skill-first; human installation is through
the GitHub Release package.

## Consequences

- Documentation and Skills lead with `code-intel`; they mention
  `archive/code-intel.ps1` only for installation and recovery.
- The official GitHub repository is the sole remote trust root for v0.5.
- The launcher exposes concise human output, a machine-readable JSON mode, and
  stable failure exit codes without weakening digest verification.
