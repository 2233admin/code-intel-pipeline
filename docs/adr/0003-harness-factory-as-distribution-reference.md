# Treat harness factories as distribution references

Accepted. Code Intel Pipeline can use MetaHarness-style systems as a future packaging and distribution reference for branded CLIs, host bundles, release gates, SBOMs, and provenance. It must not depend on a harness factory at scanner runtime: scanner, artifact consumer, Agent Goal Intake, and harness packaging remain separate layers so repository evidence stays stable while distribution choices can change later.
