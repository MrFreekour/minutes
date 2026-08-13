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
});
