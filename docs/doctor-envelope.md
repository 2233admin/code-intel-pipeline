# Doctor envelope contract

`doctor` is a registered A01 capability. It consumes exactly one A03-verified
`repository.snapshot` Artifact Ref and emits one
`code-intel-doctor-observation.v1` artifact whose Artifact Ref is bound to the
same snapshot identity. The environment policy is stored without host paths and
is independently SHA-256 bound inside the observation.

The bootstrap probe itself is native Rust
(`crates/code-intel-cli/src/doctor_bootstrap.rs`, surfaced as `code-intel doctor
bootstrap`). It emits `code-intel-doctor-bootstrap-observation.v1`, explicitly
marked `observation_only`; the adapter whitelists fields from that observation,
reconciles `orchestration/integrations.json`, and removes paths and command
output before publication. Presence, readiness, conformance, and admissibility
are separate fields. Doctor never emits engineering facts and never claims
provider admissibility.

`archive/check-code-intel-tools.ps1` is a thin forwarder onto that subcommand,
retained for the installer and rollback paths (T3, issue #48). It no longer
computes anything: a missing binary is reported as a `code-intel binary` entry
in `missing` rather than as a crash, so installers can keep reporting their own
checks. Because there is now one probe implementation instead of a script plus
an in-process fallback, the kernel path needs no `pwsh` and answers identically
on every platform.

Missing or forged Artifact Refs, invalid bootstrap JSON, or an unreadable
manifest fail as contract/runtime errors. Missing tools, nonconforming present
providers, and manifest drift are domain diagnoses: the result remains a valid
completed envelope with the observation artifact, `verdict=fail`, and
`exitCode=10`. This preserves evidence without converting failure into success.

The A09 DAG executes `repo.snapshot -> doctor` alongside the existing snapshot
to inventory branch. Direct shell invocation remains a non-authoritative
bootstrap/rollback path until E09 approves retirement of that production branch.
