#!/usr/bin/env node

/**
 * MCP HTTP Transport Integration Tests
 *
 * Starts the built server in `--transport http` mode and drives it over real
 * HTTP. Covers:
 *  1. initialize + tools/list round trip via the MCP SDK client
 *  2. the HTTP surface matches stdio exactly (tools and resources) — the
 *     per-session server factory is what could silently drop registrations,
 *     so parity against a stdio round trip is the check that matters
 *  3. two concurrent sessions in one process, each with its own session id
 *  4. session teardown releases server-side state
 *  5. localhost hardening: cross-origin POSTs, non-JSON bodies, unknown
 *     session ids, and non-initialize requests without a session are refused
 *  6. --help lists the transport flags and exits 0
 *
 * Requires the debug CLI at target/debug/minutes (the server refuses to start
 * without it, on either transport).
 *
 * Run: node crates/mcp/test/mcp_http_transport_test.mjs
 */

import { execFileSync, spawn } from "child_process";
import { join } from "path";
import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { StreamableHTTPClientTransport } from "@modelcontextprotocol/sdk/client/streamableHttp.js";
import { StdioClientTransport } from "@modelcontextprotocol/sdk/client/stdio.js";

const MCP_DIR = join(import.meta.dirname, "..");
const SERVER_ENTRY = join(MCP_DIR, "dist", "index.js");
const STARTUP_TIMEOUT_MS = 30000;

let passed = 0;
let failed = 0;

// Async-aware runner. The sibling suite (mcp_tools_test.mjs) is deliberately
// synchronous; reusing its helper here would resolve the callback's promise
// after PASS was already printed, swallowing every failure.
const pending = [];
function test(name, fn) {
  pending.push({ name, fn });
}

async function runTests() {
  for (const { name, fn } of pending) {
    try {
      await fn();
      console.log(`  PASS: ${name}`);
      passed++;
    } catch (e) {
      console.error(`  FAIL: ${name} — ${e.message}`);
      failed++;
    }
  }
}

function assert(condition, msg) {
  if (!condition) throw new Error(msg || "assertion failed");
}

function assertEqual(actual, expected, msg) {
  if (actual !== expected)
    throw new Error(msg || `expected ${expected}, got ${actual}`);
}

/** Start the server on an OS-assigned port and read the port back from stderr. */
function startHttpServer() {
  const child = spawn(process.execPath, [SERVER_ENTRY, "--transport", "http", "--port", "0"], {
    stdio: ["ignore", "pipe", "pipe"],
    env: { ...process.env, RUST_LOG: "error" },
  });

  return new Promise((resolvePromise, rejectPromise) => {
    let stderr = "";
    const timer = setTimeout(() => {
      child.kill("SIGKILL");
      rejectPromise(
        new Error(`server did not report a listening port in ${STARTUP_TIMEOUT_MS}ms:\n${stderr}`)
      );
    }, STARTUP_TIMEOUT_MS);

    child.stderr.on("data", (chunk) => {
      stderr += chunk.toString();
      const match = stderr.match(/listening on (http:\/\/[^\s]+)/);
      if (match) {
        clearTimeout(timer);
        resolvePromise({ child, url: match[1], stderrSoFar: () => stderr });
      }
    });
    child.once("exit", (code) => {
      clearTimeout(timer);
      rejectPromise(new Error(`server exited early (code ${code}):\n${stderr}`));
    });
  });
}

async function stopChild(child) {
  if (child.exitCode !== null || child.signalCode !== null) return;
  child.kill("SIGTERM");
  await new Promise((resolvePromise) => {
    const timer = setTimeout(() => {
      child.kill("SIGKILL");
      resolvePromise();
    }, 5000);
    child.once("exit", () => {
      clearTimeout(timer);
      resolvePromise();
    });
  });
}

async function connectHttpClient(url, name) {
  const client = new Client({ name, version: "1.0.0" });
  await client.connect(new StreamableHTTPClientTransport(new URL(url)));
  return client;
}

/** POST a raw JSON-RPC body, bypassing the SDK client, to probe the guards. */
async function rawPost(url, { body, headers = {}, method = "POST" } = {}) {
  const response = await fetch(url, {
    method,
    headers: {
      "Content-Type": "application/json",
      Accept: "application/json, text/event-stream",
      ...headers,
    },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const text = await response.text();
  return { status: response.status, text };
}

const INITIALIZE_BODY = {
  jsonrpc: "2.0",
  id: 1,
  method: "initialize",
  params: {
    protocolVersion: "2025-06-18",
    capabilities: {},
    clientInfo: { name: "raw-probe", version: "1.0.0" },
  },
};

console.log("MCP HTTP Transport Integration Tests\n");

// Both children probe the CLI's capability list with a 2s spawnSync budget and
// gate optional tools on the result. A cold debug binary can blow that budget
// in one child and not the other, which reads as surface drift. One warm-up
// invocation makes the probe deterministic.
try {
  execFileSync(
    join(import.meta.dirname, "..", "..", "..", "target", "debug", "minutes"),
    ["capabilities"],
    { encoding: "utf-8", timeout: 30000, stdio: ["ignore", "pipe", "pipe"] }
  );
} catch {
  // Older CLIs have no `capabilities` subcommand; the probe then agrees anyway.
}

const server = await startHttpServer();
const baseUrl = server.url.replace(/\/mcp$/, "");

let httpClient;
let httpTools;
let httpResources;

try {
  test("initialize + tools/list round trip over HTTP", async () => {
    httpClient = await connectHttpClient(server.url, "http-primary");
    const info = httpClient.getServerVersion();
    assertEqual(info?.name, "minutes", "server name should be minutes");

    httpTools = (await httpClient.listTools()).tools;
    assert(Array.isArray(httpTools), "tools should be an array");
    assert(httpTools.length > 0, "server should expose tools over HTTP");

    httpResources = (await httpClient.listResources()).resources;
    assert(Array.isArray(httpResources), "resources should be an array");
  });

  test("HTTP tool and resource surface matches stdio exactly", async () => {
    const stdioClient = new Client({ name: "stdio-parity", version: "1.0.0" });
    const stdioTransport = new StdioClientTransport({
      command: process.execPath,
      args: [SERVER_ENTRY],
      env: { ...process.env, RUST_LOG: "error" },
      stderr: "ignore",
    });
    await stdioClient.connect(stdioTransport);
    try {
      const stdioTools = (await stdioClient.listTools()).tools
        .map((t) => t.name)
        .sort();
      const stdioResources = (await stdioClient.listResources()).resources
        .map((r) => r.uri)
        .sort();
      const overHttpTools = httpTools.map((t) => t.name).sort();
      const overHttpResources = httpResources.map((r) => r.uri).sort();

      assert(stdioTools.length > 0, "stdio should expose tools");
      assertEqual(
        overHttpTools.join(","),
        stdioTools.join(","),
        `tool surface drift.\n  stdio only: ${stdioTools.filter((n) => !overHttpTools.includes(n))}\n  http only: ${overHttpTools.filter((n) => !stdioTools.includes(n))}`
      );
      assertEqual(
        overHttpResources.join(","),
        stdioResources.join(","),
        `resource surface drift.\n  stdio only: ${stdioResources.filter((u) => !overHttpResources.includes(u))}\n  http only: ${overHttpResources.filter((u) => !stdioResources.includes(u))}`
      );
      // Spot-check a few known registrations so an empty-vs-empty match can
      // never be mistaken for parity.
      for (const name of ["get_status", "list_meetings", "resummarize_meeting", "knowledge_status"]) {
        assert(overHttpTools.includes(name), `${name} must be served over HTTP`);
      }
    } finally {
      await stdioClient.close();
    }
  });

  test("two concurrent clients get independent sessions", async () => {
    const second = await connectHttpClient(server.url, "http-secondary");
    try {
      const secondTools = (await second.listTools()).tools;
      assertEqual(
        secondTools.length,
        httpTools.length,
        "a second session should see the same tool count"
      );
      assert(
        second.transport.sessionId &&
          second.transport.sessionId !== httpClient.transport.sessionId,
        "each client should get its own session id"
      );
      const health = await rawPost(`${baseUrl}/health`, { method: "GET" });
      const parsed = JSON.parse(health.text);
      assertEqual(parsed.sessions, 2, "server should report two live sessions");
    } finally {
      // close() only tears down the client side; DELETE is what ends the
      // server-side session.
      await second.transport.terminateSession();
      await second.close();
    }
  });

  test("closing a session releases it server-side", async () => {
    // Teardown is driven by the transport's onclose, so poll rather than
    // assume the DELETE response and the server-side cleanup are ordered.
    let sessions = -1;
    for (let attempt = 0; attempt < 20; attempt++) {
      const health = await rawPost(`${baseUrl}/health`, { method: "GET" });
      sessions = JSON.parse(health.text).sessions;
      if (sessions === 1) break;
      await new Promise((r) => setTimeout(r, 100));
    }
    assertEqual(sessions, 1, "only the primary session should remain");
  });

  test("get_status is callable over HTTP", async () => {
    const result = await httpClient.callTool({ name: "get_status", arguments: {} });
    assert(Array.isArray(result.content), "tool result should carry content");
    assert(result.content.length > 0, "get_status should return text content");
  });

  test("cross-origin POST is rejected", async () => {
    const { status, text } = await rawPost(server.url, {
      body: INITIALIZE_BODY,
      headers: { Origin: "http://evil.example" },
    });
    assertEqual(status, 403, "a non-loopback Origin must be refused");
    assert(text.includes("cross-origin"), `expected a cross-origin error, got: ${text}`);
  });

  test("loopback Origin is accepted", async () => {
    const { status } = await rawPost(server.url, {
      body: INITIALIZE_BODY,
      headers: { Origin: "http://localhost:5173" },
    });
    assertEqual(status, 200, "a loopback Origin must still be served");
  });

  test("non-JSON content type is rejected", async () => {
    const response = await fetch(server.url, {
      method: "POST",
      headers: { "Content-Type": "text/plain", Accept: "application/json, text/event-stream" },
      body: JSON.stringify(INITIALIZE_BODY),
    });
    assertEqual(response.status, 415, "text/plain POSTs must be refused");
  });

  test("unknown session id is rejected", async () => {
    const { status } = await rawPost(server.url, {
      body: { jsonrpc: "2.0", id: 2, method: "tools/list", params: {} },
      headers: { "Mcp-Session-Id": "00000000-0000-0000-0000-000000000000" },
    });
    assertEqual(status, 404, "an unknown session id must be refused");
  });

  test("non-initialize request without a session is rejected", async () => {
    const { status, text } = await rawPost(server.url, {
      body: { jsonrpc: "2.0", id: 3, method: "tools/list", params: {} },
    });
    assertEqual(status, 400, "a sessionless tools/list must be refused");
    assert(text.includes("Mcp-Session-Id"), `expected a session hint, got: ${text}`);
  });

  test("unknown path is not served", async () => {
    const { status } = await rawPost(`${baseUrl}/`, { method: "GET" });
    assertEqual(status, 404, "only /mcp and /health are served");
  });

  test("--help documents the transport flags and exits 0", async () => {
    const { code, stdout } = await new Promise((resolvePromise) => {
      const child = spawn(process.execPath, [SERVER_ENTRY, "--help"], {
        stdio: ["ignore", "pipe", "pipe"],
      });
      let out = "";
      child.stdout.on("data", (chunk) => (out += chunk.toString()));
      child.once("exit", (exitCode) => resolvePromise({ code: exitCode, stdout: out }));
    });
    assertEqual(code, 0, "--help should exit 0");
    for (const fragment of ["--transport <stdio|http>", "--port", "--host", "127.0.0.1"]) {
      assert(stdout.includes(fragment), `--help output should mention ${fragment}`);
    }
  });

  await runTests();
} finally {
  if (httpClient) {
    try {
      await httpClient.close();
    } catch {
      // The server may already be gone; teardown is best-effort.
    }
  }
  await stopChild(server.child);
}

console.log(`\nResults: ${passed} passed, ${failed} failed, ${passed + failed} total`);
process.exit(failed > 0 ? 1 : 0);
