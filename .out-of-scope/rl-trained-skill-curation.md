# Non-goal: RL-trained skill-document curation (SkillRise-style)

## Non-goal

Adopting or integrating [SkillRise](https://github.com/Within-yao/SkillRise) — or any RL
training stack of its family (verl / verl-agent / GRPO-style trainers) — to train or host
skill-document curation behavior. No code reuse, no vendored trainer, no gym-style task
environments in this repo.

## Why

SkillRise is an end-to-end RL framework (verl fork: Python + Ray + vLLM + FSDP/Megatron)
that trains a small policy to alternate Solve and Curate over ordered task sequences,
rewarding the Curate step by gamma-discounted downstream task success. Evaluated
2026-08-02: this pipeline is a Rust CLI for repository understanding, structural gates,
and artifact handoff — it trains no models, has no GPU stack, and its evolving-document
needs (skill docs, memory, handoff artifacts) are already served by plain curation.
SkillRise's only increment over that practice is *training* the curate behavior via RL,
which is inseparable from its training stack. Serves neither north-star axis (AI fast
code understanding; write less unnecessary output), so per the yardstick it is cut.

## Instead

- Keep the one transferable idea without the stack: judge a distilled/curated document by
  downstream task success, not by direct doc-quality review. Landed as the docs
  acceptance arm proposal in #93 and the reclassification follow-up in #104.
- If cross-task skill distillation ever becomes a product need, it is a prompting/eval
  concern on existing artifacts, not a training concern.
