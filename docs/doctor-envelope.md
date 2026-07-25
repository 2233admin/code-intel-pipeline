# Doctor envelope contract

`doctor` is a registered A01 capability. It consumes exactly one A03-verified
`repository.snapshot` Artifact Ref and emits one
`code-intel-doctor-observation.v1` artifact whose Artifact Ref is bound to the
same snapshot identity. The environment policy is stored without host paths and
is independently SHA-256 bound inside the observation.

`check-code-intel-tools.ps1` remains the shell-compatible fresh-machine probe.
Its JSON is explicitly marked `observation_only`; the Rust adapter whitelists
fields from that probe, reconciles `orchestration/integrations.json`, and removes
paths and command output before publication. Presence, readiness, conformance,
and admissibility are separate fields. Doctor never emits engineering facts and
never claims provider admissibility.

Missing or forged Artifact Refs, invalid bootstrap JSON, or an unreadable
manifest fail as contract/runtime errors. Missing tools, nonconforming present
providers, and manifest drift are domain diagnoses: the result remains a valid
completed envelope with the observation artifact, `verdict=fail`, and
`exitCode=10`. This preserves evidence without converting failure into success.

The A09 DAG executes `repo.snapshot -> doctor` alongside the existing snapshot
to inventory branch. Direct shell invocation remains a non-authoritative
bootstrap/rollback path until E09 approves retirement of that production branch.
