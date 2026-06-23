# Use Ponytail as Implementation Minimalism Benchmark

Accepted.

Code Intel Pipeline uses Ponytail as a reference benchmark for implementation minimalism: choose the smallest sufficient implementation before writing code, starting with no code and repository reuse before standard library, platform-native features, installed dependencies, one-liners, or new local implementation.

Ponytail is a benchmark, not runtime dependency. Do not install Ponytail for Code Intel Pipeline, do not call it from the scanner, and do not make Agent Goal Intake, Harness Factory Reference, artifact contracts, tests, or CI depend on Ponytail runtime behavior.

The benchmark is intentionally behavioral guidance for agents and maintainers. It protects Code Intel Pipeline from accumulating new orchestration layers while preserving verification, error handling, security, accessibility, data-loss prevention, and artifact contract boundaries.
