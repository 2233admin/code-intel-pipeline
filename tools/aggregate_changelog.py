#!/usr/bin/env python3
"""Aggregate changelog.d fragments into CHANGELOG.md (towncrier-style).

Usage:
  python tools/aggregate_changelog.py --dry-run
  python tools/aggregate_changelog.py --version 0.7.0-beta.6 --dry-run
  python tools/aggregate_changelog.py --version 0.7.0-beta.6
  python tools/aggregate_changelog.py --check-pr --base-ref origin/main

Fragment files live in changelog.d/*.md (README.md and dotfiles are skipped).
Each file declares a type (feat/fix/docs/...) then Keep-a-Changelog bullets.
"""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
from collections import defaultdict
from dataclasses import dataclass
from datetime import date
from pathlib import Path

# Force UTF-8 on stdio so Chinese fragments and Keep-a-Changelog em-dashes
# (U+2014) never get re-encoded as the host locale (cp1252 on en-US Windows
# runners). Without this, a child Python writing to a pipe uses the console
# code page and the parent dies with UnicodeDecodeError on 0x97.
for _stream in (sys.stdout, sys.stderr):
    try:
        _stream.reconfigure(encoding="utf-8")
    except (AttributeError, OSError, ValueError):
        # Older Python / closed streams / non-TextIO wrappers: leave alone.
        pass


ROOT = Path(__file__).resolve().parents[1]
DEFAULT_FRAGMENTS_DIR = ROOT / "changelog.d"
DEFAULT_CHANGELOG = ROOT / "CHANGELOG.md"

# type token -> Keep a Changelog subsection title
TYPE_TO_SECTION: dict[str, str] = {
    "feat": "Added",
    "feature": "Added",
    "added": "Added",
    "add": "Added",
    "fix": "Fixed",
    "fixed": "Fixed",
    "bug": "Fixed",
    "bugfix": "Fixed",
    "change": "Changed",
    "changed": "Changed",
    "refactor": "Changed",
    "docs": "Changed",
    "doc": "Changed",
    "documentation": "Changed",
    "remove": "Removed",
    "removed": "Removed",
    "deprecate": "Removed",
    "deprecated": "Removed",
    "security": "Security",
    "sec": "Security",
    "note": "Notes",
    "notes": "Notes",
    "misc": "Notes",
    "chore": "Notes",
}

# Preferred section order when rendering a new version block.
SECTION_ORDER = ("Added", "Fixed", "Changed", "Removed", "Security", "Notes")

SKIP_NAMES = frozenset({"readme.md", ".gitkeep"})

TYPE_LINE_RE = re.compile(r"(?i)^\s*type\s*:\s*([a-z][a-z0-9_-]*)\s*$")
FRONTMATTER_TYPE_RE = re.compile(
    r"(?is)\A---\s*\n(?:.*\n)*?type\s*:\s*([a-z][a-z0-9_-]*)\s*\n(?:.*\n)*?---\s*\n(.*)\Z"
)
HEADING_VERSION_RE = re.compile(r"^## \[([^\]]+)\](?:\s+—\s+(.+))?\s*$", re.M)
SECTION_HEADING_RE = re.compile(r"^### (.+?)\s*$", re.M)


@dataclass(frozen=True)
class Fragment:
    path: Path
    section: str
    body: str  # bullet lines, no trailing blank-line padding required


def repo_root_from(path: Path) -> Path:
    return path.resolve()


def is_fragment_file(path: Path) -> bool:
    if not path.is_file():
        return False
    if path.name.startswith("."):
        return False
    if path.name.lower() in SKIP_NAMES:
        return False
    return path.suffix.lower() == ".md"


def list_fragments(fragments_dir: Path) -> list[Path]:
    if not fragments_dir.is_dir():
        return []
    return sorted(
        (p for p in fragments_dir.iterdir() if is_fragment_file(p)),
        key=lambda p: p.name.lower(),
    )


def parse_fragment(path: Path) -> Fragment:
    text = path.read_text(encoding="utf-8")
    # Normalize newlines early so body joins are stable across platforms.
    text = text.replace("\r\n", "\n").replace("\r", "\n")

    type_token: str | None = None
    body = text

    fm = FRONTMATTER_TYPE_RE.match(text)
    if fm:
        type_token = fm.group(1).lower()
        body = fm.group(2)
    else:
        lines = text.split("\n")
        # First non-empty line may be `type: feat`
        for i, line in enumerate(lines):
            if not line.strip():
                continue
            m = TYPE_LINE_RE.match(line)
            if m:
                type_token = m.group(1).lower()
                body = "\n".join(lines[i + 1 :])
            break

    if type_token is None:
        # Filename form: 174.feat.md
        parts = path.stem.split(".")
        if len(parts) >= 2 and parts[-1].lower() in TYPE_TO_SECTION:
            type_token = parts[-1].lower()
            body = text

    if type_token is None:
        raise ValueError(
            f"{path.as_posix()}: missing type (use `type: feat` line, YAML "
            f"frontmatter, or a `name.feat.md` filename)"
        )

    section = TYPE_TO_SECTION.get(type_token)
    if section is None:
        known = ", ".join(sorted(TYPE_TO_SECTION))
        raise ValueError(
            f"{path.as_posix()}: unknown type {type_token!r}; expected one of: {known}"
        )

    body = body.strip("\n")
    # Drop a single leading blank line after the type marker.
    body = body.lstrip("\n")
    if not body.strip():
        raise ValueError(f"{path.as_posix()}: fragment body is empty")

    # Ensure each non-empty content line is a markdown bullet or continuation.
    normalized_lines: list[str] = []
    for line in body.split("\n"):
        stripped = line.rstrip()
        if not stripped:
            # Keep blank lines between bullets only if already present mid-body.
            if normalized_lines and normalized_lines[-1] != "":
                normalized_lines.append("")
            continue
        if stripped.startswith(("-", "*", "+")) or stripped.startswith(" "):
            normalized_lines.append(stripped)
        else:
            # Promote a bare paragraph line into a bullet.
            normalized_lines.append(f"- {stripped}")

    # Trim trailing blank lines.
    while normalized_lines and normalized_lines[-1] == "":
        normalized_lines.pop()

    return Fragment(path=path, section=section, body="\n".join(normalized_lines))


def load_fragments(fragments_dir: Path) -> list[Fragment]:
    fragments: list[Fragment] = []
    for path in list_fragments(fragments_dir):
        fragments.append(parse_fragment(path))
    return fragments


def group_by_section(fragments: list[Fragment]) -> dict[str, list[Fragment]]:
    grouped: dict[str, list[Fragment]] = defaultdict(list)
    for frag in fragments:
        grouped[frag.section].append(frag)
    return grouped


def render_section_block(section: str, fragments: list[Fragment]) -> str:
    lines = [f"### {section}", ""]
    for frag in fragments:
        lines.append(frag.body)
        lines.append("")
    return "\n".join(lines).rstrip() + "\n"


def render_version_body(grouped: dict[str, list[Fragment]]) -> str:
    parts: list[str] = []
    for section in SECTION_ORDER:
        if section not in grouped:
            continue
        parts.append(render_section_block(section, grouped[section]))
    # Any unexpected section names (should not happen with TYPE_TO_SECTION).
    for section in sorted(grouped):
        if section in SECTION_ORDER:
            continue
        parts.append(render_section_block(section, grouped[section]))
    return "\n".join(parts).rstrip() + "\n"


def find_heading_span(text: str, version_label: str) -> tuple[int, int] | None:
    """Return [start, end) byte offsets of the `## [label]` section body span.

    `start` is the index of the heading line; `end` is the start of the next
    `## [` heading or EOF.
    """
    matches = list(HEADING_VERSION_RE.finditer(text))
    for i, m in enumerate(matches):
        if m.group(1) == version_label:
            start = m.start()
            end = matches[i + 1].start() if i + 1 < len(matches) else len(text)
            return start, end
    return None


def merge_into_section_body(existing_body: str, grouped: dict[str, list[Fragment]]) -> str:
    """Merge fragment bullets into an existing version/Unreleased body.

    `existing_body` is everything after the `## [...]` heading line, up to
    (but not including) the next `## [` heading. It may start with a blank
    line and optional prose before the first `###` subsection.
    """
    # Split off the heading line is already done by caller; body may include
    # leading newline after heading.
    text = existing_body
    if text.startswith("\n"):
        # Keep a single leading newline convention later.
        pass

    # Locate existing ### subsections.
    matches = list(SECTION_HEADING_RE.finditer(text))
    if not matches:
        # No subsections yet: preamble (if any) + new sections.
        preamble = text.rstrip("\n")
        new_body = render_version_body(grouped)
        if preamble.strip():
            return preamble.rstrip() + "\n\n" + new_body
        return "\n" + new_body if not new_body.startswith("\n") else new_body

    # Build map section -> (start_of_heading, end_of_section)
    spans: dict[str, tuple[int, int]] = {}
    order: list[str] = []
    for i, m in enumerate(matches):
        name = m.group(1).strip()
        start = m.start()
        end = matches[i + 1].start() if i + 1 < len(matches) else len(text)
        spans[name] = (start, end)
        order.append(name)

    preamble = text[: matches[0].start()]
    # Work on a mutable list of (section_name | None for preamble, content)
    # Simpler: rebuild from preamble + each section, appending fragments.

    pieces: list[str] = [preamble]
    seen: set[str] = set()

    for name in order:
        start, end = spans[name]
        section_text = text[start:end]
        # section_text includes `### Name\n` and following content.
        if name in grouped:
            # Append fragment bodies before the trailing whitespace of the section.
            core = section_text.rstrip("\n")
            addition = "\n".join(f.body for f in grouped[name])
            section_text = core + "\n" + addition + "\n\n"
            seen.add(name)
        pieces.append(section_text)

    # Append brand-new sections in SECTION_ORDER.
    for name in SECTION_ORDER:
        if name in grouped and name not in seen:
            pieces.append(render_section_block(name, grouped[name]) + "\n")
            seen.add(name)
    for name in sorted(grouped):
        if name not in seen:
            pieces.append(render_section_block(name, grouped[name]) + "\n")

    result = "".join(pieces)
    # Normalize: at most one trailing newline at EOF for this body slice —
    # caller stitches headings; ensure body ends with \n\n before next ## if
    # we consumed up to next heading (existing text already had that).
    if not result.endswith("\n"):
        result += "\n"
    return result


def apply_fragments_to_changelog(
    changelog_text: str,
    grouped: dict[str, list[Fragment]],
    *,
    version: str | None,
    release_date: str | None,
) -> str:
    text = changelog_text.replace("\r\n", "\n").replace("\r", "\n")
    if not text.endswith("\n"):
        text += "\n"

    if version is None:
        target = "Unreleased"
        span = find_heading_span(text, target)
        if span is None:
            raise ValueError("CHANGELOG.md has no ## [Unreleased] section")
        start, end = span
        heading_line_end = text.find("\n", start)
        if heading_line_end < 0:
            heading_line_end = len(text)
        else:
            heading_line_end += 1  # include newline
        heading = text[start:heading_line_end]
        body = text[heading_line_end:end]
        new_body = merge_into_section_body(body, grouped)
        return text[:start] + heading + new_body + text[end:]

    # Release mode: insert a new version section after Unreleased (or replace
    # empty Unreleased merge — we always create a dedicated version heading).
    version_heading = f"## [{version}]"
    if release_date:
        version_heading += f" — {release_date}"
    version_heading += "\n"

    new_block = version_heading + "\n" + render_version_body(grouped)
    if not new_block.endswith("\n"):
        new_block += "\n"

    # If the version already exists, merge into it.
    existing = find_heading_span(text, version)
    if existing is not None:
        start, end = existing
        heading_line_end = text.find("\n", start)
        if heading_line_end < 0:
            heading_line_end = len(text)
        else:
            heading_line_end += 1
        heading = text[start:heading_line_end]
        body = text[heading_line_end:end]
        new_body = merge_into_section_body(body, grouped)
        return text[:start] + heading + new_body + text[end:]

    # Insert after Unreleased block when present; else after the intro.
    unreleased = find_heading_span(text, "Unreleased")
    if unreleased is not None:
        _, ure_end = unreleased
        # Ensure blank line between Unreleased body and new version.
        prefix = text[:ure_end].rstrip("\n") + "\n\n"
        suffix = text[ure_end:].lstrip("\n")
        if suffix and not suffix.startswith("## "):
            # Should not happen; still be safe.
            return prefix + new_block + "\n" + suffix
        return prefix + new_block + "\n" + suffix

    # No Unreleased: insert after first horizontal rule / title block.
    # Fall back to prepending after the first blank line following the H1 area.
    m = re.search(r"\n## \[", text)
    if m:
        return text[: m.start()] + "\n" + new_block + text[m.start() :]
    return text.rstrip("\n") + "\n\n" + new_block


def format_preview(
    grouped: dict[str, list[Fragment]],
    *,
    version: str | None,
    release_date: str | None,
    fragment_paths: list[Path],
) -> str:
    lines: list[str] = []
    label = version if version else "Unreleased"
    heading = f"## [{label}]"
    if version and release_date:
        heading += f" — {release_date}"
    lines.append(f"# dry-run: would write into {heading}")
    lines.append("")
    lines.append(render_version_body(grouped).rstrip())
    lines.append("")
    lines.append("# fragments that would be deleted after a successful apply:")
    for p in fragment_paths:
        lines.append(f"#   - {p.as_posix()}")
    if not fragment_paths:
        lines.append("#   (none)")
    lines.append("")
    return "\n".join(lines)


def run_git(args: list[str], *, cwd: Path) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        ["git", *args],
        cwd=cwd,
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
        check=False,
    )


def check_pr_advisory(
    *,
    repo: Path,
    base_ref: str,
    head_ref: str,
) -> int:
    """Advisory: changed crates/** or orchestration/** without changelog.d fragment.

    Always exits 0. Prints a GitHub Actions warning annotation when the
    advisory condition is met so CI surfaces the hint without blocking.
    """
    diff = run_git(
        ["diff", "--name-only", f"{base_ref}...{head_ref}"],
        cwd=repo,
    )
    if diff.returncode != 0:
        # Fall back to three-dot against merge-base when the refspec fails
        # (shallow clones, missing remote-tracking branch).
        mb = run_git(["merge-base", base_ref, head_ref], cwd=repo)
        if mb.returncode != 0:
            print(
                f"changelog fragment check: could not diff {base_ref}...{head_ref} "
                f"({diff.stderr.strip() or diff.stdout.strip()}); skipping advisory.",
                file=sys.stderr,
            )
            return 0
        base_sha = mb.stdout.strip()
        diff = run_git(["diff", "--name-only", f"{base_sha}...{head_ref}"], cwd=repo)
        if diff.returncode != 0:
            print(
                "changelog fragment check: git diff failed; skipping advisory.",
                file=sys.stderr,
            )
            return 0

    files = [line.strip().replace("\\", "/") for line in diff.stdout.splitlines() if line.strip()]
    touches_gated = any(
        f.startswith("crates/") or f.startswith("orchestration/") for f in files
    )
    if not touches_gated:
        print(
            "changelog fragment check: no crates/** or orchestration/** changes; ok."
        )
        return 0

    has_fragment = any(
        f.startswith("changelog.d/")
        and not f.rstrip("/").endswith("changelog.d")
        and Path(f).name.lower() not in SKIP_NAMES
        and not Path(f).name.startswith(".")
        and f.lower().endswith(".md")
        for f in files
    )
    if has_fragment:
        print("changelog fragment check: changelog.d fragment present; ok.")
        return 0

    msg = (
        "PR changes crates/** or orchestration/** but adds no changelog.d "
        "fragment. Add changelog.d/<pr-or-slug>.md (see changelog.d/README.md). "
        "This is advisory only and does not fail the job."
    )
    # GitHub Actions annotation (ignored as plain text elsewhere).
    print(f"::warning::changelog fragment missing — {msg}")
    print(f"ADVISORY: {msg}")
    return 0


def build_parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(
        description="Aggregate changelog.d fragments into CHANGELOG.md"
    )
    p.add_argument(
        "--repo",
        type=Path,
        default=ROOT,
        help="repository root (default: parent of tools/)",
    )
    p.add_argument(
        "--fragments-dir",
        type=Path,
        default=None,
        help="fragments directory (default: <repo>/changelog.d)",
    )
    p.add_argument(
        "--changelog",
        type=Path,
        default=None,
        help="CHANGELOG.md path (default: <repo>/CHANGELOG.md)",
    )
    p.add_argument(
        "--version",
        default=None,
        help="version label to write (omit to target [Unreleased])",
    )
    p.add_argument(
        "--date",
        default=None,
        help="release date YYYY-MM-DD (default: today when --version is set)",
    )
    p.add_argument(
        "--dry-run",
        action="store_true",
        help="print the section that would be written; do not modify files",
    )
    p.add_argument(
        "--check-pr",
        action="store_true",
        help="advisory PR check (always exit 0); use with --base-ref/--head-ref",
    )
    p.add_argument(
        "--base-ref",
        default="origin/main",
        help="git base ref for --check-pr (default: origin/main)",
    )
    p.add_argument(
        "--head-ref",
        default="HEAD",
        help="git head ref for --check-pr (default: HEAD)",
    )
    return p


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    repo = repo_root_from(args.repo)
    fragments_dir = (
        args.fragments_dir if args.fragments_dir is not None else repo / "changelog.d"
    )
    changelog_path = (
        args.changelog if args.changelog is not None else repo / "CHANGELOG.md"
    )

    if args.check_pr:
        return check_pr_advisory(
            repo=repo, base_ref=args.base_ref, head_ref=args.head_ref
        )

    try:
        fragments = load_fragments(fragments_dir)
    except ValueError as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 2

    if not fragments:
        print("no changelog.d fragments to aggregate.", file=sys.stderr)
        return 0

    grouped = group_by_section(fragments)
    paths = [f.path for f in fragments]
    release_date = args.date
    if args.version and not release_date:
        release_date = date.today().isoformat()

    if args.dry_run:
        # Prefer printing relative paths when under repo.
        rel_paths: list[Path] = []
        for p in paths:
            try:
                rel_paths.append(p.resolve().relative_to(repo.resolve()))
            except ValueError:
                rel_paths.append(p)
        sys.stdout.write(
            format_preview(
                grouped,
                version=args.version,
                release_date=release_date,
                fragment_paths=rel_paths,
            )
        )
        return 0

    if not changelog_path.is_file():
        print(f"error: changelog not found: {changelog_path}", file=sys.stderr)
        return 2

    original = changelog_path.read_text(encoding="utf-8")
    try:
        updated = apply_fragments_to_changelog(
            original,
            grouped,
            version=args.version,
            release_date=release_date,
        )
    except ValueError as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 2

    changelog_path.write_text(updated, encoding="utf-8", newline="\n")
    for path in paths:
        path.unlink()
        print(f"removed {path}")
    print(f"updated {changelog_path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
