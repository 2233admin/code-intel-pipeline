# pon-inspired multilingual Code Evidence mapping

The reusable part of pon is not Python syntax. It is the separation between a language frontend, a normalized internal representation, and a conformance oracle. Code Intel Pipeline maps that method onto its existing `evidence.native-code` boundary without importing pon runtime or compiler code.

## Owned mapping

The Pipeline's language-neutral structural representation is the existing Code Evidence v1 tuple:

- `files.json`: source identity, language, size, and content digest;
- `symbols.json`: normalized declarations such as function, class, interface, and enum;
- `symbol-chunks.json`: declaration containment;
- `imports.json`: normalized import target observations;
- `coverage.json`: supported heuristics and explicit unknowns.

Python, JavaScript, TypeScript, Rust, Go, Java, and PowerShell adapters emit the same field shapes. Consumers can therefore rank and navigate code without branching on source syntax. The multilingual fixture is a ratchet over that shared contract: a supported language cannot silently disappear, change fact shape, or fabricate additional facts without a reviewed fixture change.

## Precision boundary

This is a structural fact envelope, not a claim of full language semantics. The current native producer is line-heuristic and explicitly reports:

- symbol precision: heuristic;
- import precision: heuristic;
- relationship precision: unknown;
- call graph: unknown.

AST shape, types, overload resolution, dynamic dispatch, control flow, effects, macro expansion, and runtime behavior belong to future parser-backed language adapters. Unsupported languages remain present in `files.json` and `chunks.json`, appear in `unsupportedFiles`, and produce no invented symbols or imports.

## Provenance and exclusions

Design reference: `can1357/pon`, revision `ab9067dbd2899c64c4d67a4bc27b8ad49472b126`. The upstream repository had no declared license when reviewed, so this work independently specifies and tests the architectural method. No pon implementation code, fixtures, or prose were copied.

The proof deliberately does not add another IR, a pon dependency, Python runtime behavior, a compiler backend, or automatic floor updates. The existing Code Evidence contract remains the owned compatibility boundary.

## Next widening step

Choose one language from measured demand, add a parser-backed adapter behind `evidence.native-code`, and prove it preserves the shared fact contract while improving a pinned precision/recall corpus. Java import extraction and C# declaration extraction remain explicit candidate gaps rather than implicit support claims.
