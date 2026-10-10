"""Behavioral controls for the public replay CLI at its Node queue boundary."""
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
DRIVER = ROOT / "tests/test_merge_queue_guard.py"

CONTROLLED_QUEUE = r"""
const fs = require('node:fs');
const {execFileSync} = require('node:child_process');
if (process.argv[2] === '--version') {
  console.log('controlled-queue-regression');
  process.exit(0);
}
if (process.argv[2] !== 'check-push') throw new Error('Expected public check-push');
const input = fs.readFileSync(0, 'utf8');
const ref = input.trim() ? input.trim().split(/\s+/)[2] : '';
if (ref && ref !== 'refs/heads/main' && ref !== 'refs/heads/codex/code-intel-atomic-model') {
  throw new Error('Unexpected destination ref');
}
const sample = !ref ? 'empty-input' : ref === 'refs/heads/main' ? 'protected-main' : 'protected-integration';
// Git housekeeping is not a source mutation, including during clean controls.
execFileSync('git', ['update-index', '--refresh']);
fs.writeFileSync('.git/replay-regression-state', sample);
let mutated = false;
if (sample === target) {
  if (operation === 'modify') {
    const before = fs.readFileSync('acceptance.py', 'utf8');
    fs.appendFileSync('acceptance.py', '\n# controlled mutation\n');
    mutated = fs.readFileSync('acceptance.py', 'utf8') !== before;
  }
  if (operation === 'delete') {
    fs.unlinkSync('acceptance.py');
    mutated = !fs.existsSync('acceptance.py');
  }
  if (operation === 'add') {
    fs.mkdirSync('src');
    fs.writeFileSync('src/unexpected.py', 'raise SystemExit(0)\n');
    mutated = fs.readFileSync('src/unexpected.py', 'utf8') === 'raise SystemExit(0)\n';
  }
  if (operation === 'acceptance') {
    fs.writeFileSync('acceptance-invoked', 'invoked');
  }
}
const exitCode = ref ? 1 : 0;
fs.appendFileSync(evidence, JSON.stringify({
  sample, exitCode, mutated,
  acceptanceInvoked: fs.existsSync('acceptance-invoked'),
}) + '\n');
process.exit(exitCode);
"""


class MergeQueueGuardRegressionTests(unittest.TestCase):
    def run_driver(self, operation, target=""):
        with tempfile.TemporaryDirectory(prefix="cip-queue-control-") as directory:
            queue = Path(directory) / "controlled-queue.cjs"
            evidence = Path(directory) / "queue-evidence.jsonl"
            queue.write_text(
                f"const operation = {json.dumps(operation)};\n"
                f"const target = {json.dumps(target)};\n"
                f"const evidence = {json.dumps(str(evidence))};\n" + CONTROLLED_QUEUE,
                encoding="utf-8",
            )
            result = subprocess.run(
                [sys.executable, str(DRIVER), "--queue-bin", str(queue)],
                cwd=ROOT, capture_output=True, text=True, encoding="utf-8", timeout=120,
            )
            observed = [
                json.loads(line)
                for line in evidence.read_text(encoding="utf-8").splitlines()
            ] if evidence.is_file() else []
            return result, observed

    def test_clean_queue_is_accepted_despite_git_bookkeeping(self):
        result, observed = self.run_driver("clean")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(len(observed), 3)
        self.assertEqual({item["sample"]: item for item in observed}, {
            "empty-input": {
                "sample": "empty-input", "exitCode": 0,
                "mutated": False, "acceptanceInvoked": False,
            },
            "protected-main": {
                "sample": "protected-main", "exitCode": 1,
                "mutated": False, "acceptanceInvoked": False,
            },
            "protected-integration": {
                "sample": "protected-integration", "exitCode": 1,
                "mutated": False, "acceptanceInvoked": False,
            },
        })

    def test_source_mutations_are_rejected(self):
        for operation, sample, exit_code in (
            ("modify", "empty-input", 0),
            ("delete", "protected-main", 1),
            ("add", "protected-integration", 1),
        ):
            with self.subTest(operation=operation, sample=sample):
                result, observed = self.run_driver(operation, sample)
                self.assertEqual(
                    [item for item in observed if item["sample"] == sample],
                    [{
                        "sample": sample, "exitCode": exit_code,
                        "mutated": True, "acceptanceInvoked": False,
                    }],
                    result.stdout + result.stderr,
                )
                self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_acceptance_marker_is_rejected(self):
        result, observed = self.run_driver("acceptance", "empty-input")
        self.assertEqual(observed, [{
            "sample": "empty-input", "exitCode": 0,
            "mutated": False, "acceptanceInvoked": True,
        }], result.stdout + result.stderr)
        self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)


if __name__ == "__main__":
    unittest.main()
