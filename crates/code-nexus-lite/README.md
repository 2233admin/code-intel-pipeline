# code-nexus-lite incubator

`code-nexus-lite` is currently demoted from the shipped workspace.

The released Code Intel Pipeline still produces `codenexus-context.json` through
the PowerShell artifact layer. This directory is kept only as an incubator note
for a future cross-platform iii worker.

Why it is demoted:

- The worker was not part of the shipped CLI path.
- The previous prototype pulled `iii-sdk`, whose current dependency chain kept
  `opentelemetry_sdk <= 0.32.0` in the repository dependency graph.
- Dependabot reported CVE-2026-48504 / GHSA-w9wp-h8wv-79jx against those lockfiles.

To revive this worker, recreate a fresh Cargo package only after upstream
`iii-sdk` resolves the vulnerable OpenTelemetry dependency chain, then add it
back to the root workspace with CI coverage.
