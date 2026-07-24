#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="${1:-${ROOT}/target/sidekick-eval/live-sidekick-engine-eval.json}"
MEDIA_OUT="${2:-${ROOT}/target/sidekick-eval/live-sidekick-media-eval.json}"

cd "${ROOT}"
cargo run --quiet -p minutes-core --no-default-features \
  --example live_sidekick_engine_eval -- --out "${OUT}"
cargo run --quiet -p minutes-core --features whisper \
  --example live_sidekick_media_eval -- --out "${MEDIA_OUT}"
