/**
 * Streamable HTTP transport for the Minutes MCP server.
 *
 * Opt-in alternative to stdio (`minutes-mcp --transport http`). One long-lived
 * process can serve several MCP clients at once instead of each host spawning
 * its own stdio subprocess.
 *
 * Session model: the MCP SDK gives one `Protocol` instance one transport
 * (`Protocol.connect()` throws on a second), and a stateless Streamable HTTP
 * transport refuses to be reused across requests because concurrent clients
 * would collide on JSON-RPC request ids. So each `initialize` gets its own
 * transport plus its own `McpServer`, keyed by the `Mcp-Session-Id` header.
 * The instances share all module-level state in index.ts — only protocol
 * state is per-session.
 *
 * Security: always binds 127.0.0.1, with no flag to change it. Binding to
 * loopback alone does not make the endpoint private, since any page in the
 * user's browser can POST to localhost, so requests are additionally checked
 * for a loopback `Host`, a loopback-or-absent `Origin`, and a JSON content
 * type. There is no authentication. Reaching the endpoint from another machine
 * is a reverse-proxy job: point one at 127.0.0.1, rewrite `Host` to the
 * upstream address so the header check passes, and put authentication on the
 * proxy.
 */

import { createServer as createNodeHttpServer } from "node:http";
import type {
  IncomingMessage,
  Server as NodeHttpServer,
  ServerResponse,
} from "node:http";
import type { Socket } from "node:net";
import { randomUUID } from "node:crypto";

import { StreamableHTTPServerTransport } from "@modelcontextprotocol/sdk/server/streamableHttp.js";
import { isInitializeRequest } from "@modelcontextprotocol/sdk/types.js";
import type { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";

/**
 * The bind address, not a default: there is no flag or environment variable to
 * change it. HTTP mode has no authentication, so a non-loopback bind would hand
 * the whole tool surface (transcripts, recording control) to anything that can
 * route to this machine. Exposing the endpoint deliberately is a reverse-proxy
 * job, where authentication can be added in front of it.
 */
export const HTTP_BIND_HOST = "127.0.0.1";

/**
 * Unassigned by IANA and outside the ranges dev servers habitually grab
 * (3000/5173/8000/8080), so a shared Minutes endpoint is unlikely to collide.
 */
export const DEFAULT_HTTP_PORT = 7373;

/** The single MCP endpoint. GET opens the notification stream, DELETE ends a session. */
export const MCP_HTTP_PATH = "/mcp";

/** Liveness/inspection endpoint — not part of the MCP protocol. */
export const HEALTH_HTTP_PATH = "/health";

/** Concurrent sessions allowed. Unauthenticated localhost, so this is bounded. */
export const DEFAULT_MAX_SESSIONS = 16;

/** Request bodies are JSON-RPC envelopes, never audio; paths are passed by name. */
export const MAX_REQUEST_BODY_BYTES = 4 * 1024 * 1024;

const LOOPBACK_HOSTNAMES = new Set(["localhost", "127.0.0.1", "::1", "[::1]"]);

export type MinutesServerFactory = () => {
  server: McpServer;
  dispose: () => void;
};

export type MinutesHttpServerOptions = {
  /** Port to bind. 0 asks the OS for a free port; read it back from `port`. */
  port?: number;
  /** Reject new sessions past this many concurrent ones. */
  maxSessions?: number;
  /** Builds a fresh McpServer per session. */
  createServer: MinutesServerFactory;
  /** Diagnostics sink. Defaults to stderr. */
  log?: (message: string) => void;
};

export type MinutesHttpServer = {
  host: string;
  /** The actually bound port — meaningful when 0 was requested. */
  port: number;
  url: string;
  sessionCount: () => number;
  close: () => Promise<void>;
};

/** Strip a `:port` suffix and IPv6 brackets from a Host/Origin authority. */
function hostnameOf(authority: string): string | null {
  const trimmed = authority.trim();
  if (!trimmed) return null;
  if (trimmed.startsWith("[")) {
    const end = trimmed.indexOf("]");
    if (end === -1) return null;
    return trimmed.slice(0, end + 1).toLowerCase();
  }
  const colon = trimmed.indexOf(":");
  return (colon === -1 ? trimmed : trimmed.slice(0, colon)).toLowerCase();
}

/**
 * DNS-rebinding defense: a hostile name resolved to 127.0.0.1 still sends its
 * own name in `Host`, so only loopback names pass. A reverse proxy fronting
 * this endpoint has to rewrite `Host` to the upstream address, which is the
 * point — reaching it under any other name is the attack.
 */
export function isAllowedHostHeader(hostHeader: string | undefined): boolean {
  if (!hostHeader) return false;
  const hostname = hostnameOf(hostHeader);
  if (!hostname) return false;
  return LOOPBACK_HOSTNAMES.has(hostname);
}

/**
 * CSRF defense: native MCP clients send no `Origin`, browsers always do on a
 * cross-origin POST. An `Origin` naming anything but loopback is a web page
 * reaching for the local server, which is exactly what must not work.
 */
export function isAllowedOrigin(origin: string | undefined): boolean {
  if (origin === undefined) return true;
  if (origin === "null") return false;
  let parsed: URL;
  try {
    parsed = new URL(origin);
  } catch {
    return false;
  }
  return LOOPBACK_HOSTNAMES.has(parsed.hostname.toLowerCase());
}

/** A `text/plain` POST is a CORS "simple request" and skips preflight. */
export function isAllowedContentType(contentType: string | undefined): boolean {
  if (!contentType) return false;
  return contentType.split(";")[0].trim().toLowerCase() === "application/json";
}

function headerValue(
  req: IncomingMessage,
  name: string
): string | undefined {
  const raw = req.headers[name];
  if (Array.isArray(raw)) return raw[0];
  return raw ?? undefined;
}

function sendJson(res: ServerResponse, status: number, body: unknown): void {
  const payload = JSON.stringify(body);
  res.writeHead(status, {
    "Content-Type": "application/json",
    "Content-Length": Buffer.byteLength(payload),
  });
  res.end(payload);
}

function sendJsonRpcError(
  res: ServerResponse,
  status: number,
  code: number,
  message: string
): void {
  sendJson(res, status, {
    jsonrpc: "2.0",
    error: { code, message },
    id: null,
  });
}

/** Read a bounded request body. Rejects (rather than buffers) oversized posts. */
function readBody(req: IncomingMessage): Promise<Buffer> {
  return new Promise((resolvePromise, rejectPromise) => {
    const chunks: Buffer[] = [];
    let total = 0;
    req.on("data", (chunk: Buffer) => {
      total += chunk.length;
      if (total > MAX_REQUEST_BODY_BYTES) {
        rejectPromise(new Error("Request body too large"));
        req.destroy();
        return;
      }
      chunks.push(chunk);
    });
    req.on("end", () => resolvePromise(Buffer.concat(chunks)));
    req.on("error", rejectPromise);
  });
}

type SessionEntry = {
  transport: StreamableHTTPServerTransport;
  server: McpServer;
  dispose: () => void;
  closing: boolean;
};

/**
 * Start the Streamable HTTP listener. Resolves once the socket is bound, with
 * the effective port filled in (relevant when port 0 was requested).
 */
export async function startMinutesHttpServer(
  options: MinutesHttpServerOptions
): Promise<MinutesHttpServer> {
  const requestedPort = options.port ?? DEFAULT_HTTP_PORT;
  const maxSessions = options.maxSessions ?? DEFAULT_MAX_SESSIONS;
  const log = options.log ?? ((message: string) => console.error(message));

  const sessions = new Map<string, SessionEntry>();
  const sockets = new Set<Socket>();

  async function closeSession(sessionId: string): Promise<void> {
    const entry = sessions.get(sessionId);
    if (!entry || entry.closing) return;
    entry.closing = true;
    sessions.delete(sessionId);
    try {
      entry.dispose();
    } catch {
      // Best-effort teardown; a wedged poller must not block the close.
    }
    try {
      await entry.server.close();
    } catch {
      // Same: the session is gone either way.
    }
    log(`[Minutes] MCP HTTP session closed: ${sessionId}`);
  }

  async function openSession(
    req: IncomingMessage,
    res: ServerResponse,
    body: unknown
  ): Promise<void> {
    if (sessions.size >= maxSessions) {
      sendJsonRpcError(
        res,
        503,
        -32000,
        `Too many concurrent MCP sessions (limit ${maxSessions}). Close an existing client or raise --max-sessions.`
      );
      return;
    }

    const instance = options.createServer();
    const transport = new StreamableHTTPServerTransport({
      sessionIdGenerator: () => randomUUID(),
      onsessioninitialized: (sessionId: string) => {
        sessions.set(sessionId, {
          transport,
          server: instance.server,
          dispose: instance.dispose,
          closing: false,
        });
        log(`[Minutes] MCP HTTP session opened: ${sessionId}`);
      },
      onsessionclosed: (sessionId: string) => {
        void closeSession(sessionId);
      },
    });

    // Chained by Protocol.connect(), so this still fires after connect.
    transport.onclose = () => {
      const sessionId = transport.sessionId;
      if (sessionId) void closeSession(sessionId);
    };

    try {
      await instance.server.connect(transport);
    } catch (error) {
      instance.dispose();
      throw error;
    }
    await transport.handleRequest(req, res, body);
  }

  async function handle(
    req: IncomingMessage,
    res: ServerResponse
  ): Promise<void> {
    if (!isAllowedHostHeader(headerValue(req, "host"))) {
      sendJsonRpcError(res, 403, -32000, "Forbidden: unrecognized Host header");
      return;
    }
    if (!isAllowedOrigin(headerValue(req, "origin"))) {
      sendJsonRpcError(
        res,
        403,
        -32000,
        "Forbidden: cross-origin requests are not accepted"
      );
      return;
    }

    const path = (req.url ?? "/").split("?")[0];

    if (path === HEALTH_HTTP_PATH) {
      if (req.method !== "GET") {
        sendJsonRpcError(res, 405, -32000, "Method not allowed");
        return;
      }
      sendJson(res, 200, {
        ok: true,
        transport: "http",
        sessions: sessions.size,
        max_sessions: maxSessions,
      });
      return;
    }

    if (path !== MCP_HTTP_PATH) {
      sendJsonRpcError(
        res,
        404,
        -32000,
        `Not found. The MCP endpoint is ${MCP_HTTP_PATH}`
      );
      return;
    }

    let body: unknown;
    if (req.method === "POST") {
      if (!isAllowedContentType(headerValue(req, "content-type"))) {
        sendJsonRpcError(
          res,
          415,
          -32000,
          "Unsupported Media Type: expected application/json"
        );
        return;
      }
      let raw: Buffer;
      try {
        raw = await readBody(req);
      } catch {
        sendJsonRpcError(res, 413, -32000, "Request body too large");
        return;
      }
      try {
        body = JSON.parse(raw.toString("utf-8"));
      } catch {
        sendJsonRpcError(res, 400, -32700, "Parse error: body is not JSON");
        return;
      }
    }

    const sessionId = headerValue(req, "mcp-session-id");
    if (sessionId) {
      const entry = sessions.get(sessionId);
      if (!entry) {
        sendJsonRpcError(res, 404, -32001, "Session not found");
        return;
      }
      await entry.transport.handleRequest(req, res, body);
      return;
    }

    if (req.method === "POST" && isInitializeRequest(body)) {
      await openSession(req, res, body);
      return;
    }

    sendJsonRpcError(
      res,
      400,
      -32000,
      "Bad Request: Mcp-Session-Id header required (send initialize first)"
    );
  }

  const httpServer: NodeHttpServer = createNodeHttpServer((req, res) => {
    handle(req, res).catch((error) => {
      log(
        `[Minutes] MCP HTTP request failed: ${error instanceof Error ? error.message : String(error)}`
      );
      if (!res.headersSent) {
        sendJsonRpcError(res, 500, -32603, "Internal server error");
      } else {
        res.end();
      }
    });
  });

  // SSE streams hold connections open; without this, close() never resolves.
  httpServer.on("connection", (socket: Socket) => {
    sockets.add(socket);
    socket.on("close", () => sockets.delete(socket));
  });

  await new Promise<void>((resolvePromise, rejectPromise) => {
    const onError = (error: Error) => rejectPromise(error);
    httpServer.once("error", onError);
    httpServer.listen(requestedPort, HTTP_BIND_HOST, () => {
      httpServer.removeListener("error", onError);
      resolvePromise();
    });
  });

  const address = httpServer.address();
  const boundPort =
    typeof address === "object" && address !== null ? address.port : requestedPort;
  return {
    host: HTTP_BIND_HOST,
    port: boundPort,
    url: `http://${HTTP_BIND_HOST}:${boundPort}${MCP_HTTP_PATH}`,
    sessionCount: () => sessions.size,
    close: async () => {
      for (const sessionId of Array.from(sessions.keys())) {
        await closeSession(sessionId);
      }
      for (const socket of Array.from(sockets)) {
        socket.destroy();
      }
      sockets.clear();
      await new Promise<void>((resolvePromise) => {
        httpServer.close(() => resolvePromise());
      });
    },
  };
}
