#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
FIXTURE="$(mktemp -d "${TMPDIR:-/tmp}/codenexus-bench.XXXXXX")"
OUT="$FIXTURE/context.json"
trap 'rm -rf "$FIXTURE"' EXIT

mkdir -p "$FIXTURE/src" "$FIXTURE/src/target" "$FIXTURE/src/.git" "$FIXTURE/src/node_modules"
cat > "$FIXTURE/src/large.rs" <<'EOF'
// deterministic large source fixture
pub fn alpha() { println!("alpha"); }
pub fn beta() { println!("beta"); }
pub fn gamma() { println!("gamma"); }
pub fn delta() { println!("delta"); }
pub fn epsilon() { println!("epsilon"); }
pub fn zeta() { println!("zeta"); }
pub fn eta() { println!("eta"); }
pub fn theta() { println!("theta"); }
EOF
cat > "$FIXTURE/src/medium.ts" <<'EOF'
export function medium() {
  return "medium";
}
EOF
cat > "$FIXTURE/src/small.py" <<'EOF'
def small():
    return "small"
EOF
cat > "$FIXTURE/src/target/generated.rs" <<'EOF'
pub fn generated() { panic!("must be excluded"); }
EOF
cat > "$FIXTURE/src/.git/ignored.rs" <<'EOF'
pub fn ignored() { panic!("must be excluded"); }
EOF
cat > "$FIXTURE/src/node_modules/vendor.js" <<'EOF'
module.exports = "must be excluded";
EOF

cargo run --quiet --release --manifest-path "$ROOT/Cargo.toml" -- \
  codenexus generate \
  --repo "$FIXTURE" \
  --target "$FIXTURE/src/.." \
  --out "$OUT" \
  --observed-at 0 \
  --max-files 2 \
  --max-references-per-file 2

node - "$OUT" <<'NODE'
const fs = require('fs');
const file = process.argv[2];
const doc = JSON.parse(fs.readFileSync(file, 'utf8'));
const selected = (doc.files || []).map((entry) => entry.path);
const refs = (doc.files || []).reduce((sum, entry) => sum + (entry.references || []).length, 0);
const forbidden = selected.filter((name) => /(^|\/)(work|artifact|artifacts|staging|\.code-intel|\.git|node_modules|target|dist|build|\.venv|__pycache__)(\/|$)/i.test(name));
const path_rendering = selected[0] === 'src/large.rs' && (doc.files || []).every((entry) =>
  (entry.references || []).every((reference) => !reference.includes('//?/') && !reference.includes('\\?\\')),
);
const checks = [
  doc.tool === 'codenexus-lite',
  doc.generatedAt === '1970-01-01T00:00:00.000Z',
  selected.length === 2,
  path_rendering,
  forbidden.length === 0,
  refs <= 4,
];
if (refs > 4) throw new Error(`reference bound exceeded: ${refs}`);
const quality = checks.filter(Boolean).length;
console.log(`METRIC codenexus_context_quality=${quality}`);
console.log(`METRIC codenexus_generated_path_leaks=${forbidden.length}`);
console.log(`METRIC codenexus_context_files=${selected.length}`);
console.log(`METRIC codenexus_reference_matches=${refs}`);
console.log(`METRIC codenexus_context_bytes=${fs.statSync(file).size}`);
NODE
