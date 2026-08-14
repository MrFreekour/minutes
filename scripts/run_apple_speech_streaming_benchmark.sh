#!/bin/bash
set -euo pipefail

AUDIO="${1:-crates/assets/demo.wav}"
ITERATIONS="${2:-10}"
CHUNK_MS="${3:-120}"
PRESET="${4:-volatile-no-punctuation}"
OUTPUT="$(mktemp -d)/minutes-apple-speech-streaming-benchmark"

swiftc -parse-as-library -O scripts/benchmark_apple_speech_streaming.swift \
  -o "$OUTPUT" \
  -framework Foundation \
  -framework AVFAudio \
  -framework CoreMedia \
  -framework Speech

values=()
for ((iteration = 1; iteration <= ITERATIONS; iteration++)); do
  report="$($OUTPUT "$AUDIO" "$CHUNK_MS" "$PRESET" 2>/dev/null)"
  value="$(jq -r '.firstUsefulMs // empty' <<<"$report")"
  if [[ -z "$value" ]]; then
    echo "iteration $iteration emitted no useful partial" >&2
    exit 1
  fi
  values+=("$value")
  jq -c --argjson iteration "$iteration" '{iteration: $iteration, firstUsefulMs, eventCount, maxUsefulCadenceMs}' <<<"$report"
done

sorted="$(printf '%s\n' "${values[@]}" | sort -n)"
p95_index=$(( (ITERATIONS * 95 + 99) / 100 ))
p95="$(sed -n "${p95_index}p" <<<"$sorted")"
jq -n \
  --arg engine "apple-dictation-transcriber" \
  --arg preset "$PRESET" \
  --argjson iterations "$ITERATIONS" \
  --argjson chunkMs "$CHUNK_MS" \
  --argjson p95FirstUsefulPartialMs "$p95" \
  '{engine: $engine, preset: $preset, iterations: $iterations, chunkMs: $chunkMs, p95FirstUsefulPartialMs: $p95FirstUsefulPartialMs, targetMs: 700, passed: ($p95FirstUsefulPartialMs < 700)}'

if (( p95 >= 700 )); then
  exit 1
fi
