# Fixture notes

This file has no functions and no imports, deliberately -- it exists to
exercise Arm A's fallback to the whole-file `code_evidence.chunks` pointer
when neither `code_evidence.symbols` nor `code_evidence.imports` has an
entry for a file at all (e.g. an unsupported-language file, mirroring
.github/workflows/ci.yml in the real questions).
