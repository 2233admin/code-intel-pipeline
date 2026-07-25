# Staged artifact writer contract

`artifact.stage-write` is the A06 filesystem transaction boundary. It creates and validates
content-addressed artifacts under an explicitly trusted local staging authority. It does not
publish a run, write `run-complete.json`, update an index, or decide whether evidence is admissible.
Those responsibilities remain A07, A08, and A04 respectively.

## Runtime interface

The independent Rust module is `crates/code-intel-cli/src/staged_artifact.rs`. A runtime adapter
can connect it without changing the contract:

1. Call `StagedWriter::begin(authority_root, snapshot_identity)`.
2. For each already-authorized output, call `stage(bytes, ArtifactWriteContract)`. The contract
   carries the registered artifact schema, artifact type, maximum size, and payload validator.
3. Call `seal()` to receive `StagedArtifactSet`. The set owns rollback and serializes through
   `to_manifest_value()` as `code-intel-staged-artifact-set.v1`.
4. A future A07 consumer may call `prepare_for_commit()` immediately before its own atomic rename.
   This only releases held directory handles after final sync. The set still owns rollback when
   A07 fails; A06 contains no promotion or completion-marker behavior.

Every returned Artifact Ref uses `objects/sha256/<lowercase-sha256>` and binds the registered
schema, type, payload digest, and consumed snapshot identity. Repeated bytes reuse the one object
inside that owned staging tree while preserving one returned ref per requested artifact.

## Filesystem and durability invariants

- The unique tree is `<authority>/.staging/stage-<nonce>`, so temporary files and addressed objects
  are on the authority's volume. Root, shared staging, owned stage, object, and digest-directory
  handles must all report the same Windows volume serial or Unix device id. Nothing outside that
  hidden staging namespace is created.
- Authority and child directories are opened without following links. Windows keeps no-share-delete
  handles across the write. Linux creates directories and files relative to held directory file
  descriptors with `mkdirat`/`openat(O_NOFOLLOW)`.
- Bytes are bounded and payload-schema validated before the first write. The temporary file is
  created with create-new semantics, written completely, and flushed with `sync_all` / Windows
  `FlushFileBuffers` before publication.
- Windows publishes without replacement through `MoveFileExW(MOVEFILE_WRITE_THROUGH)`. Linux uses
  no-replace `linkat`, removes only its temporary name, and `fsync`s the object directory.
- The published object is reopened through the A03 stable no-follow reader, then its bytes, size,
  digest, payload schema, Artifact Ref path, and snapshot binding are checked before return.

## Ownership and interruption

Ownership starts only after a successful create-new or no-replace publication operation. The
writer records every owned temporary/object path together with its stable file identity and records
only directories it created uniquely. An existing nonce, child directory, or addressed object is
never inferred to be owned from its path or matching bytes.

Rollback verifies the recorded identity before deleting an owned file, then removes directories
only with non-recursive `remove_dir`. It never uses recursive tree deletion. A foreign entry keeps
the containing directories non-empty; those directories and the foreign entry are preserved and
reported as residuals. The shared `.staging` directory is never part of the ownership set.

Any `stage()` failure eagerly closes held handles and attempts owned rollback before returning the
error. A rollback failure or foreign residual is attached to the returned failure. Identity-owned
file removals that fail remain tracked for a `Drop` retry; foreign residuals are deliberately not
reclassified as owned. A failed writer cannot be sealed.

The deterministic `before_publish` proving hook can create the addressed target in the exact race
window after the owned temporary is flushed and before no-replace publication. Different bytes
produce `Collision` and a residual report; matching bytes deduplicate successfully without granting
delete ownership. In both cases subsequent rollback/drop preserves the competitor's object.
The internal `owned_by_stage` disposition reports this distinction without adding a non-standard
field to the serialized Artifact Ref. `seal()` rejects a set containing such an unowned object, so
A07 can never receive a commit candidate that would move or delete the competitor's entry.

The proving harness can stop after `StageCreated`, `TempCreated`, `FileSynced`, `ObjectPublished`,
or `DirectorySynced`. Each interruption leaves no final run and rollback removes the owned tree.
These are A06 write phases, not the A07 promotion/marker phases.

## Error boundaries

- `Contract`: invalid digest/schema/type, over-limit bytes, payload-schema failure, or incoherent ref.
- `Boundary`: linked/non-directory authority or non-portable staging component.
- `Collision`: nonce ownership collision or different bytes at an addressed object name.
- `HostIo`: permission, device, lock, flush, or other host filesystem failure.
- `Interrupted`: deterministic proving-test injection at a named write phase.

The module deliberately has no provider logic, recommendation authority, run coordinator,
publication marker, index mutation, database, or external CAS dependency.
