#!/usr/bin/env node

import { readFileSync } from "node:fs";

const files = {
  tauri: JSON.parse(readFileSync("tauri/src-tauri/tauri.macos.conf.json", "utf8")),
  build: readFileSync("scripts/build.sh", "utf8"),
  install: readFileSync("scripts/install-dev-app.sh", "utf8"),
  package: readFileSync("scripts/package-apple-speech-xpc.sh", "utf8"),
  worker: readFileSync("crates/core/src/apple_speech_worker.rs", "utf8"),
  xpc: readFileSync("crates/core/src/macos_graph_xpc.rs", "utf8"),
  swift: readFileSync("crates/core/src/macos_apple_speech_bridge.swift", "utf8"),
  main: readFileSync("tauri/src-tauri/src/main.rs", "utf8"),
  info: readFileSync(
    "crates/cli/assets/minutes-apple-speech-worker-Info.plist",
    "utf8",
  ),
  entitlements: readFileSync(
    "tauri/src-tauri/minutes-apple-speech-worker.entitlements",
    "utf8",
  ),
};

function validate(candidate) {
  const errors = [];
  const requireText = (source, value, message) => {
    if (!source.includes(value)) errors.push(message);
  };
  const forbid = (source, pattern, message) => {
    if (pattern.test(source)) errors.push(message);
  };

  if (
    !(candidate.tauri.bundle?.externalBin ?? []).includes(
      "bin/minutes-apple-speech-worker",
    )
  ) {
    errors.push("Tauri must stage the Apple Speech worker as an external binary");
  }
  for (const source of [candidate.build, candidate.install]) {
    requireText(
      source,
      "minutes-apple-speech-worker-${HOST_TARGET}",
      "every macOS build path must stage the exact Apple Speech worker",
    );
    requireText(
      source,
      "package-apple-speech-xpc.sh",
      "every macOS build path must package the Apple Speech XPC service",
    );
  }
  for (const value of [
    'SOURCE_WORKER="$APP_BUNDLE/Contents/MacOS/minutes-apple-speech-worker"',
    'XPC_BUNDLE="$APP_BUNDLE/Contents/XPCServices/com.useminutes.apple-speech-worker.xpc"',
    'test ! -e "$SOURCE_WORKER"',
    "--identifier com.useminutes.apple-speech-worker",
    'codesign --verify --strict --verbose=4 "$XPC_BUNDLE"',
    "seal_apple_speech_worker_hash.py",
  ]) {
    requireText(
      candidate.package,
      value,
      `Apple Speech packaging is missing invariant: ${value}`,
    );
  }
  requireText(
    candidate.info,
    "<string>com.useminutes.apple-speech-worker</string>",
    "the XPC Info.plist must bind the dedicated service identifier",
  );
  const entitlementKeys = [
    ...candidate.entitlements.matchAll(/<key>([^<]+)<\/key>/g),
  ].map((match) => match[1]);
  if (
    entitlementKeys.length !== 1 ||
    entitlementKeys[0] !== "com.apple.security.app-sandbox"
  ) {
    errors.push("the Apple Speech worker entitlement allowlist must be App Sandbox only");
  }

  for (const value of [
    "MINUTES_APPLE_SPEECH_WORKER_CDHASH_V1=",
    "current_process_is_trusted_distribution()",
    "peer_requirement_api_available()",
    "MAX_UTTERANCE_SECONDS",
    "samples.iter().any(|sample| !sample.is_finite())",
    "process_private_audio_request",
    "RLIMIT_NPROC",
    "RLIMIT_AS",
    "setitimer",
  ]) {
    requireText(
      candidate.worker,
      value,
      `Apple Speech authority or resource boundary is missing: ${value}`,
    );
  }
  for (const value of [
    "APPLE_SPEECH_XPC_PARENT_REQUEST_LOCK",
    "APPLE_SPEECH_XPC_SETTLEMENT_FAILED",
    "com.useminutes.apple-speech-worker",
    "open_apple_speech_authenticated_connection",
    "set_peer_requirement(connection.object, &requirement)",
    "set_command(begin.0, COMMAND_BEGIN)",
    "handle_apple_speech_service_message",
    "service_request_nonce_matches",
    "xpc_connection_send_barrier",
  ]) {
    requireText(
      candidate.xpc,
      value,
      `Apple Speech authenticated XPC boundary is missing: ${value}`,
    );
  }
  for (const value of [
    "AVAudioPCMBuffer",
    "UnsafeBufferPointer(start: samples",
    "sourceBuffer.floatChannelData",
    "SpeechAnalyzer.bestAvailableAudioFormat",
    "minutes_apple_speech_free_response",
  ]) {
    requireText(
      candidate.swift,
      value,
      `Apple Speech in-memory bridge is missing: ${value}`,
    );
  }
  for (const source of [
    candidate.worker,
    candidate.xpc,
    candidate.swift,
    candidate.package,
  ]) {
    forbid(
      source,
      /POSIX_SPAWN_START_SUSPENDED|SIGCONT|attest_and_resume/,
      "the rejected suspended-spawn primitive must not return",
    );
  }
  forbid(
    candidate.swift,
    /AVAudioFile|audioPath|temporaryDirectory|NSTemporaryDirectory/,
    "the private Apple Speech bridge must not open or create a named audio file",
  );
  forbid(
    candidate.worker,
    /Command::new|BoundExecutable|executable:|arguments:|environment:/,
    "the dedicated Apple Speech worker must not become a generic process launcher",
  );
  requireText(
    candidate.main,
    "install_apple_speech_worker_service(service)",
    "the desktop must bind the embedded authority before normal startup",
  );
  return errors;
}

if (process.argv.includes("--self-test")) {
  const mutations = [
    ["generic launcher", "worker", (value) => `${value}\nCommand::new(\"helper\");`],
    ["named audio file", "swift", (value) => `${value}\nlet f: AVAudioFile? = nil`],
    [
      "service left in MacOS",
      "package",
      (value) => value.replace('test ! -e "$SOURCE_WORKER"', 'test -e "$SOURCE_WORKER"'),
    ],
    [
      "missing peer requirement",
      "xpc",
      (value) =>
        value.replaceAll(
          "set_peer_requirement(connection.object, &requirement)",
          "drop(requirement)",
        ),
    ],
  ];
  for (const [name, key, mutate] of mutations) {
    const candidate = { ...files, [key]: mutate(files[key]) };
    if (validate(candidate).length === 0) {
      throw new Error(`self-test mutation was accepted: ${name}`);
    }
  }
  process.stdout.write("Apple Speech worker packaging self-test passed\n");
  process.exit(0);
}

const errors = validate(files);
if (errors.length > 0) {
  for (const error of errors) process.stderr.write(`ERROR: ${error}\n`);
  process.exit(1);
}
process.stdout.write("Apple Speech worker packaging checks passed\n");
