"""Normalize upstream compete view-model scores into an advisory Code Intel artifact."""

from __future__ import annotations

import argparse
import importlib.util
import json
import statistics
import sys
from datetime import datetime, timezone
from pathlib import Path


def load_build_report(scripts: Path):
    module_path = scripts / "build_report.py"
    if not module_path.is_file():
        raise FileNotFoundError(f"compete build_report.py is missing: {module_path}")
    sys.path.insert(0, str(scripts))
    spec = importlib.util.spec_from_file_location("code_intel_compete_build_report", module_path)
    if spec is None or spec.loader is None:
        raise RuntimeError("cannot load compete build_report.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def unwrap_list(value) -> list[str]:
    if isinstance(value, dict):
        value = value.get("value", [])
    return [str(item) for item in value] if isinstance(value, list) else []


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--repo", required=True, type=Path)
    parser.add_argument("--compete-scripts", required=True, type=Path)
    parser.add_argument("--data-dir", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()

    upstream = load_build_report(args.compete_scripts.resolve())
    data = upstream.load_all(args.data_dir.resolve())
    entities = upstream.build_entities(data)
    now = datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    report, enriched = upstream.synth_report(data, entities, now)
    views = upstream.build_view_models(entities, enriched)
    radar = views.get("radar", {})
    axes = radar.get("axes", [])
    own = next((item for item in radar.get("series", []) if item.get("is_self")), None)
    if own is None or len(axes) != len(own.get("scores", [])) or not axes:
        raise ValueError("compete did not produce a complete self radar series")

    scores = [float(value) for value in own["scores"]]
    result = {
        "schema": "code-intel-competitive-score.v1",
        "status": "completed",
        "authority": "advisory",
        "evidenceStatus": "derived_from_compete_datasets",
        "generatedAt": now,
        "repoPath": str(args.repo.resolve()),
        "product": own.get("name", "This product"),
        "overallScore": round(statistics.fmean(scores), 1),
        "axes": [
            {"name": str(name), "score": round(score, 1)}
            for name, score in zip(axes, scores, strict=True)
        ],
        "keyFindings": unwrap_list(report.get("executive_summary", {}).get("key_findings")),
        "source": {
            "project": "lbj96347/compete",
            "generator": "compete/build_report.py",
            "dataPath": str(args.data_dir.resolve()),
        },
        "routing": {
            "affectsHospitalScore": False,
            "affectsStructuralGate": False,
            "nextAction": "Review the InsightKit report and evidence before acting on recommendations.",
        },
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(result, ensure_ascii=False, separators=(",", ":")))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
