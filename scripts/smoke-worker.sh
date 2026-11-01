#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
scratch_dir="$(mktemp -d)"
trap 'rm -rf "$scratch_dir"' EXIT
CLOUDSTREAM_DATA_DIR="$scratch_dir/data" cargo run -p cloudstream-core -- smoke > "$scratch_dir/result.json"
python3 - "$scratch_dir/result.json" <<'PY'
import json, sys
with open(sys.argv[1]) as source:
    result = json.load(source)
assert result['ok'] is True
assert result['provider']['name'] == 'Blender Open Movies'
assert len(result['details']['episodes']) == 1
assert result['links']['streams'][0]['url'].startswith('https://download.blender.org/')
print('PASS: JVM package installation → provider registration → search → details → stream resolution')
PY
