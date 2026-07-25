# inventory.rg capability contract

`inventory.rg` v1 is the deterministic capability-envelope adapter for the existing
`run-code-intel.ps1` file-inventory step. It does not replace the compatibility facade.

Compatibility is defined over the normalized file set, not the byte order emitted by a
particular `rg --files` process. The legacy runner preserves ripgrep's process order, which is
not stable across otherwise identical invocations. The v1 capability therefore writes unique
paths in ordinal byte order. On Windows, where ripgrep emits representable UTF-8 paths, records
retain the legacy LF delimiter. On Unix, records are raw path bytes with NUL delimiters, so a path
containing a newline or non-UTF-8 byte is not trimmed, split, or otherwise rewritten. This ordering
and platform encoding are versioned materialized-view rules; they do not add or remove path values.

The adapter applies the legacy default exclusions and accepts `options.inventoryExclude` as an
ordered array of additional ripgrep glob arguments. Tests compare the normalized output of the
real PowerShell runner with the capability output, including hidden files, default exclusions,
custom exclusions, Unicode, whitespace, quotes, and shell metacharacters.

Glob evaluation is delegated to the pinned `rg` executable rather than reimplemented. The lease
creates an exclusively owned temporary manifest mirror containing empty ordinary files for the
expected inventory paths. Snapshot-controlled `.gitignore`, `.ignore`, and `.rgignore` files retain
their frozen bytes in the mirror. Symlink entries remain bound into the snapshot identity through
their target bytes, but are excluded from both inventory baselines and the artifact: the contracted
`rg --files` invocation does not use `-L`, so symlinks are never recreated or followed. The live repository is
used only for a membership baseline: ripgrep runs with `--no-ignore`, no custom globs, and only the
fixed structural exclusions plus literal, manifest-derived exclusions for every gitlink subtree,
then its normalized path set must equal the mirror's matching
no-ignore baseline. Filtering and artifact bytes come exclusively from a second mirror traversal
using frozen ignore controls plus the default and custom globs. Live ignore-file bytes therefore
cannot affect a `head_only` artifact. Post-consumption lease verification still runs before
publication. Mirror nodes are identity-anchored and removed in reverse order on success or failure.
Gitlink OIDs remain snapshot-bound, while populated submodule worktrees are neither traversed nor
materialized; ripgrep glob metacharacters in Git paths are escaped before each dynamic exclusion is
passed as a direct argument.

The v1 invocation uses `--no-require-git`, `--no-ignore-parent`, `--no-ignore-global`, and
`--no-ignore-exclude`, and removes `RIPGREP_CONFIG_PATH`. Therefore repository snapshot ignore
control files remain active even in the mirror, while parent ignore files, Git
`.git/info/exclude`, global ignore configuration, and ripgrep config files cannot become
unversioned path-selection inputs. For a non-root scope, `--no-ignore-parent` also means ignore
controls above the scope's traversal root are intentionally inactive; controls at or below that
scope remain snapshot-bound and active.

`--out` is the only write boundary. The adapter publishes the fixed relative artifact
`files.txt`; requests cannot choose an artifact path. The directory must not already exist.
An empty repository is a successful empty inventory even though ripgrep reports its normal
"no files matched" process code.

Publication creates the output directory and temporary file exclusively, rejects symlink/reparse
objects at identity checks, publishes with an exclusive hard link, and rechecks the owned directory
and file identities. Identities come from still-open handles: Unix uses device/inode and Windows
uses volume serial/file index from `GetFileInformationByHandle`, never timestamps, size, or other
forgeable metadata. Failure cleanup retains those handles as ownership anchors: Windows deletes by
handle with `SetFileInformationByHandle`, while Unix reopens the path, verifies device/inode, and
then unlinks it. A competing object is preserved, post-link failure leaves no published artifact,
and cleanup failures are included in diagnostics.
