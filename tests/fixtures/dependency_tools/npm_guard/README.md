# Real npm guard baseline

Recorded before dependency migration using the actual installed 0.5.1 package on Node 24.19.0. Acquisition retained the committed SHA512 integrity and disabled lifecycle scripts. No push or landing was performed.

Each request is the original public `check-push` argv and Git-format stdin. The main and integration requests must exit 1; empty stdin must exit 0. All tracked source fingerprints must remain unchanged. Replay consumes these request files, not reconstructed requests.

Comparison retains only `exitCode` and `trackedSourceUnchanged`. There are no volatile timestamps or random IDs. Reported provider version and incidental stdout/stderr wording are retained in `recorded.json` for provenance, not re-pinned as behavior. Candidate acquisition/version changes are reported separately.

`code-intel` capability verification remains a separate public CLI smoke. Run the real queue replay with `python tests/test_merge_queue_guard.py`; a missing executable is an error, never a skipped test. The replay uses an isolated Git repository importing the real queue configuration, with only the consequential acceptance command replaced by a marker command exiting 7. Every sample must leave that marker absent. This exposes a guard bypass without launching acceptance, Cargo, push, or landing on the isolation-excluded host.
