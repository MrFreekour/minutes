import { afterEach, describe, expect, it } from "vitest";

import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { StreamableHTTPClientTransport } from "@modelcontextprotocol/sdk/client/streamableHttp.js";
import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";

import {
  HTTP_BIND_HOST,
  DEFAULT_HTTP_PORT,
  isAllowedContentType,
  isAllowedHostHeader,
  isAllowedOrigin,
  startMinutesHttpServer,
  HEALTH_HTTP_PATH,
  MAX_REQUEST_BODY_BYTES,
} from "./httpTransport.js";
import type {
  MinutesHttpServer,
  MinutesHttpServerOptions,
  MinutesServerFactory,
} from "./httpTransport.js";

describe("http transport request guards", () => {
  it("binds loopback, with nothing to override it", () => {
    expect(HTTP_BIND_HOST).toBe("127.0.0.1");
    expect(DEFAULT_HTTP_PORT).toBeGreaterThan(1024);
  });

  describe("Host header", () => {
    it("accepts loopback names", () => {
      for (const host of [
        "127.0.0.1:7373",
        "localhost:7373",
        "localhost",
        "[::1]:7373",
      ]) {
        expect(isAllowedHostHeader(host)).toBe(true);
      }
    });

    // The allowlist is fixed, so a LAN address never passes. Before --host was
    // removed, binding one added it here, which is the hole that removal closed.
    it("rejects a non-loopback address", () => {
      expect(isAllowedHostHeader("192.168.1.10:7373")).toBe(false);
      expect(isAllowedHostHeader("0.0.0.0:7373")).toBe(false);
    });

    // A rebound hostname resolves to 127.0.0.1 but still sends its own name.
    it("rejects a rebound hostname", () => {
      expect(isAllowedHostHeader("attacker.example:7373")).toBe(false);
    });

    it("rejects a missing Host header", () => {
      expect(isAllowedHostHeader(undefined)).toBe(false);
      expect(isAllowedHostHeader("")).toBe(false);
    });
  });

  describe("Origin header", () => {
    // Native MCP clients send no Origin; browsers always do cross-origin.
    it("accepts an absent Origin", () => {
      expect(isAllowedOrigin(undefined)).toBe(true);
    });

    it("accepts loopback origins", () => {
      expect(isAllowedOrigin("http://localhost:5173")).toBe(true);
      expect(isAllowedOrigin("http://127.0.0.1:3000")).toBe(true);
    });

    it("rejects web origins and opaque origins", () => {
      expect(isAllowedOrigin("https://evil.example")).toBe(false);
      expect(isAllowedOrigin("null")).toBe(false);
      expect(isAllowedOrigin("not a url")).toBe(false);
    });
  });

  describe("Content-Type", () => {
    it("accepts JSON with parameters", () => {
      expect(isAllowedContentType("application/json")).toBe(true);
      expect(isAllowedContentType("application/json; charset=utf-8")).toBe(true);
      expect(isAllowedContentType("Application/JSON")).toBe(true);
    });

    // text/plain is a CORS simple request and would skip preflight entirely.
    it("rejects everything else", () => {
      expect(isAllowedContentType("text/plain")).toBe(false);
      expect(isAllowedContentType("application/x-www-form-urlencoded")).toBe(false);
      expect(isAllowedContentType(undefined)).toBe(false);
    });
  });
});

describe("session reclamation", () => {
  // A stub server, not the real Minutes one: this exercises the transport's
  // session bookkeeping, and importing index.ts would drag in the CLI probe.
  function stubFactory(): ReturnType<MinutesServerFactory> {
    const server = new McpServer(
      { name: "reclamation-test", version: "0.0.0" },
      { capabilities: { tools: {} } }
    );
    server.registerTool(
      "ping",
      { description: "Test tool", inputSchema: {} },
      async () => ({ content: [{ type: "text" as const, text: "pong" }] })
    );
    return { server, dispose: () => {} };
  }

  let httpServer: MinutesHttpServer | undefined;
  const clients: Client[] = [];

  afterEach(async () => {
    // Without this vitest hangs: the listener and its sockets outlive the test.
    for (const client of clients.splice(0)) {
      await client.close().catch(() => {});
    }
    await httpServer?.close();
    httpServer = undefined;
  });

  async function start(overrides: Partial<MinutesHttpServerOptions> = {}) {
    httpServer = await startMinutesHttpServer({
      port: 0,
      maxSessions: 2,
      sessionIdleTimeoutMs: 150,
      createServer: stubFactory,
      log: () => {},
      ...overrides,
    });
    return httpServer;
  }

  /** Connect a real SDK client, so the abandonment path is the real one. */
  async function connect(server: MinutesHttpServer): Promise<Client> {
    const client = new Client({ name: "test-client", version: "0.0.0" });
    await client.connect(new StreamableHTTPClientTransport(new URL(server.url)));
    clients.push(client);
    return client;
  }

  async function waitFor(
    predicate: () => boolean,
    timeoutMs = 5000
  ): Promise<void> {
    const deadline = Date.now() + timeoutMs;
    while (!predicate()) {
      if (Date.now() > deadline) throw new Error("condition never became true");
      await new Promise((resolve) => setTimeout(resolve, 10));
    }
  }

  it("reclaims a session abandoned without a DELETE", async () => {
    const server = await start();
    const client = await connect(server);
    expect(server.sessionCount()).toBe(1);

    // The SDK's close() aborts its fetches and does NOT send a DELETE, which
    // is what a crashed or disconnected client looks like to the server.
    await client.close();
    clients.length = 0;

    await waitFor(() => server.sessionCount() === 0);
    expect(server.sessionCount()).toBe(0);
  });

  // The regression the review reported: not "the count drops" but "the server
  // stops serving anyone". Every session is abandoned without a DELETE, which
  // used to fill the map permanently and 503 every later client.
  it("still accepts a new client after every session was abandoned", async () => {
    const server = await start({ maxSessions: 2 });

    for (let i = 0; i < 2; i++) {
      const client = await connect(server);
      await client.close();
    }
    clients.length = 0;

    await waitFor(() => server.sessionCount() === 0);

    const fresh = await connect(server);
    const tools = await fresh.listTools();
    expect(tools.tools.map((t) => t.name)).toContain("ping");
    expect(server.sessionCount()).toBe(1);
  });

  it("keeps a session that is still holding a stream open", async () => {
    const server = await start();
    const client = await connect(server);

    // The SDK client holds a standalone GET SSE stream for the session's life,
    // so idling well past the timeout must not reclaim it.
    await new Promise((resolve) => setTimeout(resolve, 500));

    expect(server.sessionCount()).toBe(1);
    const tools = await client.listTools();
    expect(tools.tools.map((t) => t.name)).toContain("ping");
  });

  // `isInitializeRequest` validates method and params but not `id`, so an
  // initialize sent as a JSON-RPC notification passes it. Opening a session for
  // one strands the slot: the SDK answers 202 with no `Mcp-Session-Id`, so the
  // client has no handle to use it or to DELETE it, and only the reaper frees
  // it. Repeat it and the session limit is exhausted by requests that never
  // produced a usable session.
  //
  // Asserting the 400 alone is not enough — a session opened and then refused
  // would still leak. The session count is the assertion that matters.
  it("refuses an initialize sent as a notification, without opening a session", async () => {
    const server = await start();

    const notificationInitialize = {
      jsonrpc: "2.0",
      method: "initialize",
      params: {
        protocolVersion: "2025-11-25",
        capabilities: {},
        clientInfo: { name: "no-id-client", version: "0.0.0" },
      },
    };

    for (let attempt = 0; attempt < 4; attempt += 1) {
      const response = await fetch(server.url, {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          Accept: "application/json, text/event-stream",
        },
        body: JSON.stringify(notificationInitialize),
      });
      expect(response.status).toBe(400);
      await response.arrayBuffer();
    }

    // maxSessions is 2 here, so four attempts would have exhausted it.
    expect(server.sessionCount()).toBe(0);

    // And the limit was never consumed: a real client still initializes.
    const client = await connect(server);
    const tools = await client.listTools();
    expect(tools.tools.map((t) => t.name)).toContain("ping");
  });

  // `id: null` is not a valid JSON-RPC request id either, and it takes a
  // different path through the schema than a missing `id`.
  it("refuses an initialize whose id is null", async () => {
    const server = await start();

    const response = await fetch(server.url, {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        Accept: "application/json, text/event-stream",
      },
      body: JSON.stringify({
        jsonrpc: "2.0",
        id: null,
        method: "initialize",
        params: {
          protocolVersion: "2025-11-25",
          capabilities: {},
          clientInfo: { name: "null-id-client", version: "0.0.0" },
        },
      }),
    });

    expect(response.status).toBe(400);
    await response.arrayBuffer();
    expect(server.sessionCount()).toBe(0);
  });
});

describe("oversized request bodies", () => {
  function stubFactory(): ReturnType<MinutesServerFactory> {
    const server = new McpServer(
      { name: "oversize-test", version: "0.0.0" },
      { capabilities: { tools: {} } }
    );
    return { server, dispose: () => {} };
  }

  let httpServer: MinutesHttpServer | undefined;

  afterEach(async () => {
    await httpServer?.close();
    httpServer = undefined;
  });

  // The whole point of the finding: the documented 413 has to arrive. Reading
  // the JSON body is the assertion — a resolved fetch alone would also be
  // satisfied by a response that never carried the error.
  it("answers an oversized body with a readable JSON-RPC 413", async () => {
    httpServer = await startMinutesHttpServer({
      port: 0,
      createServer: stubFactory,
      log: () => {},
    });

    const oversized = JSON.stringify({
      jsonrpc: "2.0",
      id: 1,
      method: "tools/call",
      params: { pad: "x".repeat(MAX_REQUEST_BODY_BYTES + 1024) },
    });

    const response = await fetch(httpServer.url, {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        Accept: "application/json, text/event-stream",
      },
      body: oversized,
    });

    expect(response.status).toBe(413);
    const payload = (await response.json()) as {
      jsonrpc: string;
      error: { code: number; message: string };
    };
    expect(payload.jsonrpc).toBe("2.0");
    expect(payload.error.message).toMatch(/too large/i);
    expect(payload.error.code).toBe(-32000);

    // The listener survives it, rather than the refusal taking the server down.
    const health = await fetch(
      `http://${httpServer.host}:${httpServer.port}${HEALTH_HTTP_PATH}`
    );
    expect(health.status).toBe(200);
  });

  it("still accepts a body just under the limit", async () => {
    httpServer = await startMinutesHttpServer({
      port: 0,
      createServer: stubFactory,
      log: () => {},
    });

    // Not an initialize, so this is refused on its merits (400) rather than on
    // size — which is what distinguishes "under the limit" from "rejected".
    const response = await fetch(httpServer.url, {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        Accept: "application/json, text/event-stream",
      },
      body: JSON.stringify({
        jsonrpc: "2.0",
        id: 1,
        method: "tools/list",
        params: { pad: "x".repeat(1024) },
      }),
    });

    expect(response.status).toBe(400);
    const payload = (await response.json()) as { error: { message: string } };
    expect(payload.error.message).not.toMatch(/too large/i);
  });
});
