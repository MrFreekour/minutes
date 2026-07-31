#!/usr/bin/env node

import { createHash } from "node:crypto";
import { existsSync, readFileSync } from "node:fs";

// Update only after line-by-line review of the complete authority boundary.
// These whole-file hashes prevent dead/comment-only duplicate code from
// satisfying the structural checks below.
const EXPECTED_SOURCE_SHA256 = {
  release: "986432ffb8697f36a026afa6e66a5bf20cd0861de5e428029edca0129088c19d",
  acceptance: "94b1a57515357bcd229ca57095f7cc10736d22e9a271b58994945ba8651de158",
  build: "5f6029f8e22484a55294f08de62df299e816e08f4b45278d143a008142a47ea5",
  dev: "70f7a3aa227440c5928bc51227d64fbf85176b5f966b2fcc04a15e159bbd223e",
  packageXpc: "baebd532d240fc26188c431f571d0a44f2ce1e7240c3277284c9ea9e69c1ef82",
  tauri: "8b338a848edfb2e5e07eca0f5e79761be67b1f03ada34343ef3556b21c069e40",
  entitlements: "7971da95784f3bdeb3ea257b5c1a31317731b513b16c47e2c4e588fc0ac40bae",
  xpc: "f3a48db2c10a14d69f074bc6671d05c715c93d1d39344005bf0a7851236fff24",
  graphWorker: "804b6bb314b314b8cf61d6f8cd5bfb7f2374ceaa355fc4d12b92fe39c009230e",
  authority: "31499fdcd41a1240c3568d5ffc73801243a1432e1312a2c31a3626d0766e5786",
  helperPlist: "543617b03e757520a201bd0a7751cc6aadb48cf0d6b4a44bfc9ef4323a69850f",
  helperEntry: "0efe701412d909021d6ae784eac941e7d9b9d1a0f2ee0f3144bcb15fc2b2ba18",
  cliCargo: "700a45e3f342fc22965218e7f346d128c655eba973e157af1a1a730fe708bafa",
};

const sources = {
  release: readFileSync(".github/workflows/release-macos.yml", "utf8"),
  acceptance: readFileSync(
    ".github/workflows/signed-dev-acceptance.yml",
    "utf8",
  ),
  build: readFileSync("scripts/build.sh", "utf8"),
  dev: readFileSync("scripts/install-dev-app.sh", "utf8"),
  packageXpc: readFileSync("scripts/package-graph-xpc.sh", "utf8"),
  tauri: readFileSync("tauri/src-tauri/tauri.macos.conf.json", "utf8"),
  entitlements: readFileSync(
    "tauri/src-tauri/minutes-graph-worker.entitlements",
    "utf8",
  ),
  xpc: readFileSync("crates/core/src/macos_graph_xpc.rs", "utf8"),
  graphWorker: readFileSync("crates/core/src/graph_worker.rs", "utf8"),
  authority: readFileSync(
    "crates/core/src/macos_graph_xpc_authority.swift",
    "utf8",
  ),
  helperPlist: readFileSync(
    "crates/cli/assets/minutes-graph-worker-Info.plist",
    "utf8",
  ),
  helperEntry: readFileSync(
    "crates/cli/src/bin/minutes-graph-worker.rs",
    "utf8",
  ),
  cliCargo: readFileSync("crates/cli/Cargo.toml", "utf8"),
};

function validate(candidate, checkGoldens = true) {
  const errors = [];
  const requirePattern = (source, pattern, message) => {
    if (!pattern.test(source)) errors.push(message);
  };
  const forbidPattern = (source, pattern, message) => {
    if (pattern.test(source)) errors.push(message);
  };
  const ordered = (source, fragments, message) => {
    let cursor = 0;
    for (const fragment of fragments) {
      const next = source.indexOf(fragment, cursor);
      if (next < 0) {
        errors.push(message);
        return;
      }
      cursor = next + fragment.length;
    }
  };

  if (checkGoldens) {
    for (const [name, expected] of Object.entries(EXPECTED_SOURCE_SHA256)) {
      const observed = createHash("sha256")
        .update(candidate[name])
        .digest("hex");
      if (observed !== expected) {
        errors.push(
          `complete ${name} graph-XPC authority changed; review it and update its golden hash`,
        );
      }
    }
  }

  let tauri;
  try {
    tauri = JSON.parse(candidate.tauri);
  } catch {
    errors.push("the macOS Tauri configuration must be valid JSON");
    tauri = {};
  }
  if (!(tauri.bundle?.externalBin ?? []).includes("bin/minutes-graph-worker")) {
    errors.push("Tauri must stage the graph worker for XPC packaging");
  }

  requirePattern(
    candidate.helperPlist,
    /<key>CFBundlePackageType<\/key>\s*<string>XPC!<\/string>/,
    "the graph helper must be an XPC service bundle",
  );
  requirePattern(
    candidate.helperPlist,
    /<key>XPCService<\/key>[\s\S]*?<key>RunLoopType<\/key>\s*<string>dispatch_main<\/string>/,
    "the graph XPC service must use the dispatch main loop",
  );
  const entitlementKeys = [
    ...candidate.entitlements.matchAll(/<key>([^<]+)<\/key>/g),
  ].map((match) => match[1]);
  if (
    entitlementKeys.length !== 1 ||
    entitlementKeys[0] !== "com.apple.security.app-sandbox"
  ) {
    errors.push("the graph XPC entitlement allowlist must contain only App Sandbox");
  }

  ordered(
    candidate.packageXpc,
    [
      'SOURCE_WORKER="$APP_BUNDLE/Contents/MacOS/minutes-graph-worker"',
      'XPC_BUNDLE="$APP_BUNDLE/Contents/XPCServices/com.useminutes.graph-worker.xpc"',
      'mv -f "$SOURCE_WORKER" "$XPC_EXECUTABLE"',
      'test ! -e "$SOURCE_WORKER"',
      "--identifier com.useminutes.graph-worker",
      'codesign --verify --strict --verbose=4 "$XPC_BUNDLE"',
      'codesign -dvvv "$XPC_EXECUTABLE"',
      "printf '%s\\n' \"$graph_worker_cdhash\"",
      "seal_graph_worker_hash.py",
    ],
    "the local packager must move, sign, verify, hash-bind, and seal one nested XPC service",
  );
  const xpcBundleSignatures = [
    ...candidate.packageXpc.matchAll(
      /--identifier com\.useminutes\.graph-worker[\s\S]*?--sign [^\n]+\\\n\s*"\$XPC_BUNDLE"/g,
    ),
  ].length;
  if (xpcBundleSignatures !== 2) {
    errors.push("both ad-hoc and identity paths must sign the XPC bundle itself");
  }
  for (const [name, source] of [
    ["release", candidate.release],
    ["local build", candidate.build],
    ["dev build", candidate.dev],
  ]) {
    requirePattern(
      source,
      /package-graph-xpc\.sh/,
      `${name} must use the reviewed graph XPC packager`,
    );
  }
  ordered(
    candidate.release,
    [
      "package-graph-xpc.sh",
      "Contents/XPCServices/com.useminutes.graph-worker.xpc",
      '--sign "$APPLE_SIGNING_IDENTITY" "$app"',
      'codesign --verify --strict --verbose=4 "$graph_worker_bundle"',
      "notarytool submit",
      "stapler staple",
    ],
    "release must package and verify the XPC service before outer signing and notarization",
  );
  ordered(
    candidate.acceptance,
    [
      "Contents/XPCServices/com.useminutes.graph-worker.xpc",
      'test ! -e "$app/Contents/MacOS/minutes-graph-worker"',
      "--identifier com.useminutes.graph-worker",
      '--sign "$MINUTES_DEV_SIGNING_IDENTITY" "$graph_worker_bundle"',
      'codesign -dvvv "$graph_worker"',
      "MINUTES_GRAPH_WORKER_CDHASH_V1=",
      '--sign "$MINUTES_DEV_SIGNING_IDENTITY" "$app"',
      'codesign --verify --strict --verbose=4 "$graph_worker_bundle"',
    ],
    "signed acceptance must sign the inert nested XPC bundle before sealing and signing the parent",
  );

  ordered(
    candidate.xpc,
    [
      "xpc_connection_set_peer_code_signing_requirement",
      "set_peer_requirement(connection.object, &requirement)?",
      "xpc_connection_set_event_handler(connection.object, &events)",
      "xpc_connection_resume(connection.object)",
      "set_command(begin.0, COMMAND_BEGIN)",
      "connection.send_with_reply(begin.0, deadline)?",
      "set_command(chunk.0, COMMAND_CHUNK)",
    ],
    "the parent must install the exact peer requirement and complete a content-free handshake before any private chunk",
  );
  requirePattern(
    candidate.xpc,
    /format!\("identifier \\"com\.useminutes\.graph-worker\\" and cdhash H\\"\{encoded\}\\"[\s\S]*?certificate leaf\[subject\.OU\] = \\"63TMLKT8HN\\"/,
    "trusted distribution must require the exact graph service CDHash and Team",
  );
  ordered(
    candidate.xpc,
    [
      "service_parent_requirement()",
      "set_peer_requirement(peer, &requirement)",
      "xpc_connection_set_event_handler(peer, &messages)",
      "xpc_connection_resume(peer)",
    ],
    "the XPC service must authenticate its parent before accepting messages",
  );
  requirePattern(
    candidate.xpc,
    /fn send_with_reply\([\s\S]*?receiver\s*\.recv_timeout\(remaining\)/,
    "every parent XPC round trip must be bounded by the one wall clock",
  );
  requirePattern(
    candidate.graphWorker,
    /macOS 11 and source\/ad-hoc\/default-mode builds retain product parity[\s\S]*?explicit compatibility fallback, not an atomic helper claim[\s\S]*?InProcessFallback/,
    "the older-macOS and developer compatibility path must be documented as a fallback",
  );
  ordered(
    candidate.graphWorker,
    [
      "fn choose_macos_worker_transport(",
      "if !app_managed || !trusted_distribution || !peer_requirement_available",
      "MacWorkerTransportChoice::InProcessFallback",
      "} else if cached_authority",
      "MacWorkerTransportChoice::CachedAuthority",
      "MacWorkerTransportChoice::MissingService",
      "let peer_requirement_available =",
      "choose_macos_worker_transport(",
      "match choice",
      "MacWorkerTransportChoice::InProcessFallback",
      "MacWorkerTransportChoice::CachedAuthority",
      "MacWorkerTransportChoice::EmbeddedService",
      "MacWorkerTransportChoice::MissingService",
    ],
    "fallback eligibility must be decided before any cached authority while trusted supported apps fail closed on a missing service",
  );
  requirePattern(
    candidate.graphWorker,
    /MacWorkerTransportChoice::MissingService => \{\s*Err\("policy graph XPC service is unavailable in this app installation"/,
    "a trusted supported app must fail closed when its embedded XPC service is missing",
  );
  requirePattern(
    candidate.authority,
    /SecStaticCodeCheckValidity\([\s\S]*?codeDirectoryHash\(onDiskParent\) == runningParentCodeDirectoryHash/,
    "the outer bundle and already-running parent identity must be bound",
  );
  requirePattern(
    candidate.graphWorker,
    /manifest_cdhash != embedded_cdhash/,
    "the parent must bind the XPC service CDHash resource to its embedded authority",
  );
  requirePattern(
    candidate.graphWorker,
    /fn prepare_macos_graph_xpc_worker\(\)[\s\S]*?prepare_macos_graph_xpc_worker_with_wall_clock\(WORKER_WALL_CLOCK\)[\s\S]*?setrlimit\(libc::RLIMIT_NPROC[\s\S]*?macos_virtual_size\(\)\?[\s\S]*?setrlimit\(libc::RLIMIT_AS[\s\S]*?setitimer/,
    "the XPC worker must install process, address-space, and wall-clock ceilings",
  );
  ordered(
    candidate.xpc,
    [
      "pub fn run_service_main()",
      "prepare_macos_graph_xpc_worker()",
      "let claimed = Arc::new(AtomicBool::new(false))",
      "service_parent_requirement()",
      "set_peer_requirement(peer, &requirement)",
      "let awaiting_begin",
      "claim_service_process(&message_claimed)",
      "handle_service_message(message, &message_state)",
      "xpc_connection_send_message(peer_address as XpcObject, reply.0)",
      "xpc_connection_send_barrier(",
    ],
    "the XPC service must install immutable ceilings once, admit one authenticated request, and exit after its terminal reply",
  );
  requirePattern(
    candidate.xpc,
    /let outcome = \(\|\|[\s\S]*?let settlement = connection\.settle\(outcome\.is_err\(\), deadline\);[\s\S]*?XPC_SETTLEMENT_FAILED\.store\(true, Ordering::Release\)/,
    "every acquired parent connection must settle the one-shot service before publishing success, returning an error, or admitting a successor",
  );
  requirePattern(
    candidate.xpc,
    /XPC_ERROR_CONNECTION_INTERRUPTED_SYMBOL[\s\S]*?XPC_ERROR_CONNECTION_INVALID_SYMBOL[\s\S]*?if xpc_is_connection_end\(event\)/,
    "only an exact connection-interrupted or invalid event may prove the one-shot service is gone",
  );
  requirePattern(
    candidate.xpc,
    /fn settle\(&self, abort: bool, deadline: Instant\)[\s\S]*?set_command\(message\.0, COMMAND_ABORT\)[\s\S]*?self\.wait_for_service_exit\(deadline\)/,
    "failed parent operations must send a terminal abort and then observe service exit",
  );
  requirePattern(
    candidate.xpc,
    /match begin_result \{[\s\S]*?terminal_acknowledged\.load\(Ordering::Acquire\)[\s\S]*?connection\.settle\(false, deadline\)[\s\S]*?XPC_SETTLEMENT_FAILED\.store\(true, Ordering::Release\)/,
    "a failed content-free handshake must settle or poison the one-shot transport",
  );
  requirePattern(
    candidate.xpc,
    /fn ensure_transport_available\(poisoned: &AtomicBool\)[\s\S]*?poisoned\.load\(Ordering::Acquire\)[\s\S]*?requires an application restart after an unconfirmed service exit/,
    "an unconfirmed service exit must fail closed against every later request",
  );
  requirePattern(
    candidate.xpc,
    /if command == "abort" \{\s*return service_reply\(message, phase\.abort\(\)\);/,
    "the service must turn the authenticated abort command into a terminal reply",
  );
  requirePattern(
    candidate.xpc,
    /static XPC_PARENT_REQUEST_LOCK: Mutex<\(\)>[\s\S]*?fn lock_parent_request<'a>\([\s\S]*?lock\.try_lock\(\)[\s\S]*?ensure_transport_available\(poisoned\)\?[\s\S]*?remaining\.is_zero\(\)[\s\S]*?lock_parent_request\(&XPC_PARENT_REQUEST_LOCK, &XPC_SETTLEMENT_FAILED, deadline\)\?/,
    "one parent process must serialize graph requests and recheck poison after admission under the same deadline",
  );
  requirePattern(
    candidate.xpc,
    /dispatch_queue_create[\s\S]*?parent_callback_queue\(\)[\s\S]*?xpc_connection_create\(XPC_SERVICE_NAME\.as_ptr\(\)\.cast\(\), callback_queue\)/,
    "connection events and replies must share one serial callback queue",
  );
  requirePattern(
    candidate.xpc,
    /xpc_connection_send_message_with_reply\(connection, message, reply_queue, &handler\)/,
    "XPC reply handlers must run on the connection's serial callback queue",
  );
  requirePattern(
    candidate.xpc,
    /if let Some\(nonce\)[\s\S]*?xpc_dictionary_set_data\([\s\S]*?SERVICE_NONCE_KEY[\s\S]*?bind_service_nonce\(&mut expected_nonce, service_nonce\)/,
    "every post-handshake request and reply must remain bound to the exact service nonce",
  );
  requirePattern(
    candidate.xpc,
    /fn wait_for_service_exit[\s\S]*?terminal_acknowledged\.load\(Ordering::Acquire\)[\s\S]*?invalidated[\s\S]*?recv_timeout\(remaining\)/,
    "settlement must require both a nonce-bound terminal reply and a connection-end event without assuming callback order",
  );
  requirePattern(
    candidate.xpc,
    /SERVICE_NONCE_KEY[\s\S]*?service_nonce_from_reply\(reply\.0\)[\s\S]*?bind_service_nonce/,
    "terminal acknowledgement and later connection end must be bound to one service generation",
  );
  requirePattern(
    candidate.xpc,
    /if xpc_is_connection_end\(event\) \{\s*transport_failed\.store\(true, Ordering::Release\)/,
    "every connection interruption must immediately poison transport before any relaunching request",
  );
  requirePattern(
    candidate.xpc,
    /fn settle\(&self[\s\S]*?if self\.transport_failed\.load\(Ordering::Acquire\)[\s\S]*?transport failed before terminal acknowledgement/,
    "a transport failure or service-generation mismatch must poison rather than auto-launch an abort helper",
  );
  requirePattern(
    candidate.xpc,
    /classify_service_peer_event\([\s\S]*?ServicePeerEvent::CancelPeer[\s\S]*?ServicePeerEvent::ExitProcess[\s\S]*?message_peer_was_rejected\.store\(true, Ordering::Release\)[\s\S]*?BUSY_KEY[\s\S]*?TERMINAL_KEY[\s\S]*?return;/,
    "a losing overlap must receive an authenticated nonterminal busy response and its teardown must not kill the owner",
  );
  requirePattern(
    candidate.xpc,
    /ServicePeerEvent::CancelPeer => \{\s*unsafe \{ xpc_connection_cancel\(peer_address as XpcObject\) \};\s*return;\s*\}/,
    "an unclaimed or rejected peer teardown must cancel only that peer",
  );
  requirePattern(
    candidate.xpc,
    /fn awaiting_command_can_claim\(command: Option<&str>\) -> bool \{\s*command == Some\("begin"\)\s*\}[\s\S]*?awaiting_begin && !awaiting_command_can_claim\(command\)[\s\S]*?TERMINAL_KEY[\s\S]*?return;[\s\S]*?service_request_nonce_matches\(message, &message_service_nonce\)/,
    "only begin may claim a fresh service and every later request must carry the bound process nonce",
  );

  for (const source of Object.values(candidate)) {
    forbidPattern(
      source,
      /POSIX_SPAWN_START_SUSPENDED|attest_and_resume|SIGCONT/,
      "the rejected spawn-suspension primitive must not remain in the authority boundary",
    );
  }
  if (existsSync("crates/core/src/macos_constrained_child.rs")) {
    errors.push("the rejected constrained-child implementation must be deleted");
  }
  return errors;
}

if (process.argv.includes("--self-test")) {
  const mutations = [
    ["worker left in MacOS", "packageXpc", (value) =>
      value.replace('test ! -e "$SOURCE_WORKER"', 'test -e "$SOURCE_WORKER"')],
    ["XPC bundle not signed", "packageXpc", (value) =>
      value.replaceAll('    "$XPC_BUNDLE"', '    "$XPC_EXECUTABLE"')],
    ["parent requirement installed after resume", "xpc", (value) => {
      const line = "    set_peer_requirement(connection.object, &requirement)?;\n";
      return value.replace(line, "").replace(
        "        xpc_connection_resume(connection.object);\n",
        `        xpc_connection_resume(connection.object);\n${line}`,
      );
    }],
    ["private chunk before handshake", "xpc", (value) =>
      value.replace(
        "set_command(begin.0, COMMAND_BEGIN)",
        "set_command(begin.0, COMMAND_CHUNK)",
      )],
    ["service parent unauthenticated", "xpc", (value) =>
      value.replace(
        "if set_peer_requirement(peer, &requirement).is_err()",
        "if requirement.as_bytes().is_empty()",
      )],
    ["unbounded XPC reply", "xpc", (value) =>
      value.replaceAll("recv_timeout(remaining)", "recv()")],
    ["generic service identity", "xpc", (value) =>
      value.replace(
        'identifier \\"com.useminutes.graph-worker\\" and cdhash H\\"{encoded}\\"',
        'identifier \\"com.useminutes.graph-worker\\"',
      )],
    ["fallback misrepresented", "graphWorker", (value) =>
      value.replace(
        "is an explicit compatibility fallback, not an atomic helper claim.",
        "is an atomic helper guarantee.",
      )],
    ["cached authority bypasses compatibility fallback", "graphWorker", (value) =>
      value.replace(
        "if !app_managed || !trusted_distribution || !peer_requirement_available",
        "if !app_managed || !trusted_distribution",
      )],
    ["trusted missing service falls back", "graphWorker", (value) =>
      value.replace(
        'Err("policy graph XPC service is unavailable in this app installation".into())',
        "Ok(MacWorkerTransport::InProcessFallback)",
      )],
    ["service ceilings move per request", "xpc", (value) =>
      value.replace(
        "if crate::graph_worker::prepare_macos_graph_xpc_worker().is_err()",
        "if false",
      )],
    ["service accepts concurrent requests", "xpc", (value) =>
      value.replace(
        "if awaiting_begin && !claim_service_process(&message_claimed)",
        "if awaiting_begin && false",
      )],
    ["service process remains reusable", "xpc", (value) =>
      value.replaceAll(
        "xpc_connection_send_barrier(peer_address as XpcObject, &exit_after_send)",
        "xpc_connection_send_message(peer_address as XpcObject, exit_after_send.0)",
      )],
    ["parent returns before one-shot service settlement", "xpc", (value) =>
      value.replace(
        "let settlement = connection.settle(outcome.is_err(), deadline);",
        "let settlement = Ok(());",
      )],
    ["failed parent operation skips terminal abort", "xpc", (value) =>
      value.replace(
        "connection.settle(outcome.is_err(), deadline)",
        "connection.settle(false, deadline)",
      )],
    ["failed handshake skips terminal settlement", "xpc", (value) =>
      value.replace(
        "connection.settle(false, deadline)",
        "connection.wait_for_service_exit(deadline)",
      )],
    ["unconfirmed exit does not poison successors", "xpc", (value) =>
      value.replace(
        "if poisoned.load(Ordering::Acquire)",
        "if false",
      )],
    ["generic XPC error accepted as service exit", "xpc", (value) =>
      value.replace(
        "if xpc_is_connection_end(event)",
        'if xpc_type_is(event, "error")',
      )],
    ["service ignores authenticated abort", "xpc", (value) =>
      value.replace(
        'if command == "abort" {\n        return service_reply(message, phase.abort());\n    }',
        'if command == "abort" {\n        return service_reply(message, true);\n    }',
      )],
    ["parent requests overlap", "xpc", (value) =>
      value.replace(
        "lock_parent_request(&XPC_PARENT_REQUEST_LOCK, &XPC_SETTLEMENT_FAILED, deadline)?;",
        "let _request_guard = Mutex::new(()).lock().unwrap();",
      )],
    ["waiter skips poison recheck", "xpc", (value) =>
      value.replace(
        "ensure_transport_available(poisoned)?;",
        "let _ = poisoned;",
      )],
    ["reply callbacks are unordered", "xpc", (value) =>
      value.replace(
        "xpc_connection_send_message_with_reply(connection, message, reply_queue, &handler)",
        "xpc_connection_send_message_with_reply(connection, message, std::ptr::null_mut(), &handler)",
      )],
    ["abort ignores service generation", "xpc", (value) =>
      value.replace(
        "service_request_nonce_matches(message, &message_service_nonce)",
        "true",
      )],
    ["transport failure may relaunch abort helper", "xpc", (value) =>
      value.replace(
        "if self.transport_failed.load(Ordering::Acquire)\n            && !self.terminal_acknowledged.load(Ordering::Acquire)",
        "if false",
      )],
    ["losing overlap cancels its peer", "xpc", (value) =>
      value.replace(
        "xpc_dictionary_set_bool(reply.0, BUSY_KEY.as_ptr().cast(), true)",
        "xpc_connection_cancel(peer_address as XpcObject)",
      )],
    ["losing peer disconnect kills owner", "xpc", (value) =>
      value.replace(
        "ServicePeerEvent::CancelPeer => {\n                    unsafe { xpc_connection_cancel(peer_address as XpcObject) };\n                    return;\n                }",
        "ServicePeerEvent::CancelPeer => unsafe { libc::_exit(72) },",
      )],
    ["stale abort may claim fresh helper", "xpc", (value) =>
      value.replace(
        'command == Some("begin")',
        'matches!(command, Some("begin" | "abort"))',
      )],
    ["service may fork", "graphWorker", (value) =>
      value.replace("libc::RLIMIT_NPROC", "libc::RLIMIT_NOFILE")],
    ["production service timer is test-sized or disabled", "graphWorker", (value) =>
      value.replace(
        "prepare_macos_graph_xpc_worker_with_wall_clock(WORKER_WALL_CLOCK)",
        "prepare_macos_graph_xpc_worker_with_wall_clock(Duration::ZERO)",
      )],
    ["XPC service gains network", "entitlements", (value) =>
      value.replace(
        "</dict>",
        "<key>com.apple.security.network.client</key><true/></dict>",
      )],
    ["plain executable package", "helperPlist", (value) =>
      value.replace("<string>XPC!</string>", "<string>APPL</string>")],
    ["suspension primitive restored", "xpc", (value) =>
      `${value}\n// POSIX_SPAWN_START_SUSPENDED\n`],
  ];
  for (const [name, key, mutate] of mutations) {
    const candidate = { ...sources, [key]: mutate(sources[key]) };
    if (candidate[key] === sources[key]) {
      throw new Error(`graph XPC fixture mutation did not apply: ${name}`);
    }
    if (validate(candidate, false).length === 0) {
      throw new Error(`unsafe graph XPC mutation was accepted: ${name}`);
    }
  }
} else {
  const errors = validate(
    sources,
    !process.argv.includes("--structural-only"),
  );
  if (errors.length) {
    for (const error of errors) {
      console.error(`graph-worker packaging: ${error}`);
    }
    process.exitCode = 1;
  }
}
