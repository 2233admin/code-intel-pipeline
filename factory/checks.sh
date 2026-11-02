#!/usr/bin/env bash
# code-intel-pipeline 的门禁。工厂在单子的工作树里跑它；非交互、不改文件、任一失败非零退出。
# 内容照 .github/workflows/ci.yml 的 rust-gate 与 python 层：fmt、check、全部 Rust 测试、两条跨契约测试、仓库布局与技能包测试。
# 不跑 pr-gate.yml 的 change-risk（它读 PR 元数据、要 GitHub）与 admission-policy（要读 GitHub 评论）；这两项由 GitHub 上的 fast 工作流做。
set -euo pipefail

rustup show active-toolchain >/dev/null
cargo fmt -p code-intel -- --check
cargo check --locked
cargo test -p code-intel --locked --no-fail-fast
cargo test -p code-intel --locked --test capability_exec structured_edit_plan_is_scope_bound_and_preview_only -- --exact
cargo test -p code-intel --locked --test capability_exec ast_grep_security_scan_is_scope_bound_and_advisory_only -- --exact
python3 tests/test_repository_layout.py
python3 tests/test_skill_package.py
