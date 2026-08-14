#!/bin/bash
set -euo pipefail

if [[ -z "${1:-}" ]]; then
  echo "Usage: scripts/run_dictation_partial_benchmark.sh CORPUS.json [ITERATIONS] [OUTPUT.json]" >&2
  exit 2
fi

CORPUS="$1"
ITERATIONS="${2:-10}"
OUTPUT="${3:-}"
PLUGIN="$(pwd)/crates/sherpa-plugin/target/release/libminutes_sherpa.dylib"

if [[ ! -f "$PLUGIN" ]]; then
  (cd crates/sherpa-plugin && cargo build --release)
fi

COMMAND=(cargo run --release -p minutes-core \
  --no-default-features --features whisper,streaming,engine-sherpa \
  --example dictation_partial_benchmark -- "$CORPUS" "$ITERATIONS")

if [[ -n "$OUTPUT" ]]; then
  MINUTES_SHERPA_PLUGIN="$PLUGIN" "${COMMAND[@]}" > "$OUTPUT"
  echo "Wrote $OUTPUT"
else
  MINUTES_SHERPA_PLUGIN="$PLUGIN" "${COMMAND[@]}"
fi
