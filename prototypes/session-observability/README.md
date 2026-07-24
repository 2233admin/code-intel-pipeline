# Code Intel session observability prototype

This is a throwaway logic prototype. It tests one question:

> Does combining an agent's temporal trace with Code Intel structural hotspots create a useful
> review surface without making the trace provider part of the core pipeline?

It reads a Mindwalk trace v1 and either a Code Intel `sentrux-hotspots.json` or raw Sentrux DSM
payload, joins target paths in memory, and opens a small terminal timeline. It does not persist raw
or derived session data.

## Run

```powershell
cargo run --manifest-path <repo>\prototypes\session-observability\Cargo.toml -- --trace <trace.json> --hotspots <sentrux-hotspots.json>
```

Use `n`/`p` for adjacent events, `e` for the next edit, `v` for the next verification, `x` for the
next error, `?` for help, and `q` to quit. Commands are followed by Enter so the prototype works in
ordinary terminals without a TUI dependency.

Add `--snapshot` to render one frame and exit, which is useful for smoke checks.

## Boundary being tested

- Internalize later: a provider-neutral, redacted session evidence contract; snapshot binding;
  privacy policy; derived review signals; invocation policy.
- Keep replaceable: Mindwalk session parsing, city-map UI, local server, and optional LLM analysis.
- Invocation: explicit session review, or policy-triggered review for high-risk tasks—not every scan.

## Provenance

The input trace is compatible with [cosmtrek/mindwalk](https://github.com/cosmtrek/mindwalk) trace
schema v1 at commit `e208b6b8504138843f671e031f28129b66003a67` (MIT). This prototype consumes the
published JSON shape and does not copy Mindwalk implementation code.
