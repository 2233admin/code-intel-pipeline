---
status: proposed
date: 2026-07-25
---

# Define canonical EntityRef format and parsing

This is ADR-001 of the context-and-evidence model and ADR-0012 in the
repository-wide sequence.

Code Intel needs one portable identifier for repository entities and immutable
artifacts. Graph nodes, evidence, claims, capsules, and attestations must be able
to refer to the same entity without embedding machine paths, provider-specific
objects, or live-checkout state.

EntityRef identifies an entity. It does not prove that the entity exists, that
an observation is current, or that an artifact is trusted. SnapshotSet supplies
observation time, a resolver supplies authorized location, and Evidence Ledger
events supply claims and relations.

## Decision

EntityRef v1 is a canonical ASCII URI string with two supported schemes:

```text
repo://<repository-namespace>/<repository-digest>[/<repository-path>][#symbol=<opaque-key>]
artifact://sha256/<content-digest>
```

The supported repository namespaces are the existing
`git-lineage-v1` and `content-v1` identities:

```text
repo://git-lineage-v1/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
repo://git-lineage-v1/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/src/parser.rs
repo://git-lineage-v1/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/src/parser.rs#symbol=function%3Aparse
artifact://sha256/bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb
```

`git-lineage-v1` is stable across snapshots in the same consumed lineage.
`content-v1` remains supported for unborn or unversioned repositories, but its
digest may change with repository content and therefore does not imply durable
cross-snapshot continuity.

`service://` and `contract://` are reserved names. EntityRef v1 rejects them and
all other schemes until a later ADR defines their identity semantics. Parsers
must not accept unknown schemes merely to preserve strings.

## Canonical grammar

```abnf
entity-ref      = repo-ref / artifact-ref
repo-ref        = "repo://" repo-namespace "/" digest
                  [ "/" repo-path ]
                  [ "#symbol=" symbol-key ]
artifact-ref    = "artifact://sha256/" digest
repo-namespace  = "git-lineage-v1" / "content-v1"
digest          = 64lower-hex
repo-path       = path-segment *( "/" path-segment )
path-segment    = 1*( unreserved / pct-encoded )
symbol-key      = 1*( unreserved / pct-encoded )
unreserved      = ALPHA / DIGIT / "-" / "." / "_" / "~"
pct-encoded     = "%" HEXUPPER HEXUPPER
```

Repository paths use canonical repository-relative `/` separators and preserve
case. Each decoded segment must be valid UTF-8, non-empty, and neither `.` nor
`..`. A segment must not contain `/`, `\`, NUL, or control characters.

Percent-encoded bytes use uppercase hex. Unreserved ASCII characters must not
be percent-encoded. Raw non-ASCII input is rejected rather than normalized;
callers must encode its UTF-8 bytes before parsing. This gives every EntityRef
exactly one accepted spelling without imposing Unicode normalization on
repository paths.

Queries, user information, ports, empty path segments, and fragments other than
the repo symbol selector are invalid. A symbol selector requires a repository
path. The symbol key is provider-neutral and opaque to EntityRef; line and
column coordinates are observations, not identity.

## Interface boundary

The EntityRef module exposes a small deterministic interface:

```text
parse(text) -> EntityRef | typed parse error
canonical(EntityRef) -> canonical string
kind(EntityRef) -> repo_root | repo_path | repo_symbol | artifact
```

Parsing and canonical rendering perform no filesystem, Git, index, clock, or
network access. For every accepted string `s`:

```text
canonical(parse(s)) == s
parse(canonical(ref)) == ref
```

Equality, ordering, hashing, JSON representation, and ledger endpoints use the
parsed structure's canonical string. Implementations keep scheme-specific
fields private and use standard parsing/display conventions where the language
supports them.

Resolution is a separate adapter boundary:

```text
resolve(EntityRef, SnapshotSet, authority) -> concrete location | typed resolution error
```

Parse success does not imply existence. Resolution must use the supplied
SnapshotSet and explicit repository or artifact authority; it must not silently
fall back to the live checkout, an ambient artifact root, or a network request.
Parse errors and resolution errors are separate domains.

Stable parse error codes are:

- `empty`
- `unsupported_scheme`
- `unsupported_namespace`
- `invalid_digest`
- `invalid_repo_path`
- `symbol_requires_path`
- `invalid_symbol_key`
- `unexpected_query`
- `unexpected_fragment`
- `non_canonical_encoding`
- `input_too_long`

The concrete byte limit is an implementation constant covered by tests, not
part of the durable identifier semantics.

## Relations

Relationships are not nested fields on EntityRef and are not a seventh core
object. They are immutable Evidence Ledger events whose endpoints are canonical
EntityRefs:

```json
{
  "schema": "code-intel-ledger-event.v1",
  "type": "RelationAsserted",
  "from": "repo://git-lineage-v1/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/src/parser.rs#symbol=function%3Aparse",
  "to": "artifact://sha256/bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
  "relation": "supported_by",
  "evidenceRef": "artifact://sha256/cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
  "validFrom": "snapshot-set-12",
  "validUntil": null
}
```

The event owns the relation label, evidence, and validity interval. Retraction
or invalidation appends another event; historical endpoints are never rewritten.
Graphs and adjacency indexes are rebuildable Materialized Views over the ledger.
The complete ledger event contract and schema evolution rules belong to ADR-002.

## Versioning

`repo` and `artifact` are stable scheme names. Identity-algorithm versions live
in their namespaces, such as `git-lineage-v1`. The serialized profile is
`code-intel-entity-ref.v1`.

Adding a scheme or namespace requires an ADR and is additive only when old
strings keep exactly the same meaning. Breaking grammar or interpretation
requires a new EntityRef profile or namespace. Historical refs are never
silently reinterpreted or mass-rewritten.

Existing Artifact Ref remains the trust envelope carrying schema, type,
location, digest, and consumed Snapshot Identity. EntityRef does not replace
that envelope: `artifact://sha256/<digest>` is only its portable content
identity.

## V1 scope

V1 implements:

- repository roots, repository-relative paths, and opaque symbol selectors;
- immutable artifact identities;
- strict parsing, canonical rendering, and typed errors;
- relation endpoints that reuse canonical EntityRefs.

V1 does not implement:

- service or contract identities;
- line/column identities;
- existence checks inside parsing;
- remote dereferencing;
- a public plugin ABI or dynamic scheme registry;
- automatic equivalence across renames, forks, or `content-v1` revisions.

## Alternatives considered

### Repository-only identity

`repo://<namespace>/<digest>` is smaller, but cannot name the files and symbols
needed by capsules and relation events. Adding those coordinates in every
consumer would recreate incompatible identity formats.

### Snapshot digest inside every repository EntityRef

This makes each ref an immutable observation coordinate, but duplicates
SnapshotSet and makes unchanged logical entities acquire new identities at
every snapshot. Snapshot membership therefore remains explicit context.

### Generic parsing of unknown schemes

Round-tripping future schemes appears extensible, but it admits identifiers
whose canonicalization and security rules are undefined. V1 fails closed;
adding a scheme is cheap compared with repairing ambiguous ledger identity.

### Artifact Ref as EntityRef

Artifact Ref already carries trust and location metadata, but those fields are
not artifact identity. Reusing the whole envelope would make relocation change
an entity reference and would couple graph identity to capability transport.

## Consequences

- One canonical identifier can be used by graph nodes, ledger endpoints,
  capsule references, cache keys, and external attestations.
- Identity, observation time, existence, and trust remain independently
  testable boundaries.
- Provider adapters must translate their path and symbol identifiers at
  admission instead of leaking provider-specific objects downstream.
- Renames and semantic equivalence require evidence-backed relation events;
  EntityRef does not guess them.
- The strict parser rejects superficially equivalent spellings, reducing
  duplicate nodes and ambiguous lookups.

## Verification before implementation

Before writing collectors, the first golden scenario must manually author:

1. one single-repository SnapshotSet;
2. five to ten EntityRefs covering root, paths, symbols, and artifacts;
3. relation events including support, contradiction, and structural relations;
4. one ProblemFrame reframe;
5. one agent CapsuleRequest and manually assembled CapsuleResult;
6. test or merged-PR ExternalAttestation.

The fixture must prove canonical round trips, reject every non-canonical form
listed above, rebuild its relation view deterministically, and let a fresh agent
identify the known root cause without repository-wide search. Failure means the
schema is revised before collectors are implemented.

This follows two engineering rules: claims must be falsifiable by replayable
evidence, and the system boundary must expose feedback rather than hide it.
Names and authority are not substitutes for a passing experiment.
