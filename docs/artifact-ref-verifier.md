# Artifact Ref Verifier v1

A03 implements one shared trust boundary for capability input artifacts. A consumer supplies an
artifact-root authority independently from the Artifact Ref, the expected Snapshot Identity, and
an expected schema/type contract. Successful verification returns owned payload bytes; downstream
code does not reopen the path.

Capability execution accepts `--artifact-root <directory>` when the request has non-empty
`inputs`. Artifact paths are relative to that root. Requests without inputs remain compatible and
do not require the option. A request with inputs and no explicit root fails closed.

The verifier requires the exact v1 Artifact Ref fields, a registered artifact schema/type pair, a
non-null Snapshot Identity equal to the capability request, a regular payload file, a matching
SHA-256 over raw bytes, and a payload that validates against the registered contract. The current
registry contains `code-intel-file-inventory.v1` / `inventory.files` and
`code-intel-repository-snapshot.v1` / `repository.snapshot`; additional consumers add their
contracts centrally rather than branching on provider identity.

Portable paths reject absolute, UNC, drive, device/ADS, backslash, empty, `.`, `..`, reserved
Windows names (including `CONIN$`, `CONOUT$`, and superscript device aliases), trailing-dot/space,
decomposed combining-mark spellings, duplicate, and Unicode case-colliding spellings before
filesystem access. The filesystem boundary holds the root and every parent directory handle,
opens every component no-follow (`OPEN_REPARSE_POINT` on Windows; `openat` plus `O_NOFOLLOW` on
Linux), rejects link/reparse traversal and non-regular payloads, and returns owned bytes. Two refs
to the same stable file identity, including hardlink aliases, are rejected. Moving an artifact
root does not change verification because location is not identity.

The handle authority begins at the supplied `--artifact-root`, not at the filesystem volume root.
The operating system may resolve links in ancestors above that supplied path before the verifier
opens the artifact-root handle. The artifact-root entry itself and every Artifact Ref component
below it are opened no-follow and checked for link/reparse substitution. Therefore callers that
require a link-free absolute ancestor chain must first supply an already-resolved trusted root;
the verifier does not claim that ancestors above its authority contain no links.

Contract, schema, snapshot, location-boundary, stable-identity, size-policy, payload-validation,
and digest failures exit 65. Host read/lock/I/O failures exit 74. The registered payload contracts
include the file inventory and repository snapshot JSON (with duplicate/extra-field rejection).
Provider freshness, completeness, and provenance admissibility
remain A04; artifact creation and publication remain A06.
