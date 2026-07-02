# Rust CLI reads artifacts before replacing the scanner

Accepted. The Rust `code-intel` CLI starts as an artifact consumer for existing scanner output instead of replacing the PowerShell scanner. This keeps the current evidence-producing workflow stable while giving humans and Agents a typed, testable way to resume from `report.json`, `hospital-report.json`, and related artifact files; a future scanner rewrite must preserve the artifact data contract rather than silently changing the handoff surface.
