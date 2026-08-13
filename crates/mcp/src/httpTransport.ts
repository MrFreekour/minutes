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
 * Session lifetime: a session ends on an explicit DELETE, on server shutdown,
 * or by idle reclamation. The third one is required, not a nicety. The SDK's
 * `Client.close()` aborts its fetches without sending a DELETE, and a client
 * that crashes or loses its network sends nothing at all, so nothing would
 * ever free those entries and the session limit would fill up with dead
 * clients. See `reapIdleSessions` for what counts as idle.
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

/**
 * How much of an oversized body to read and throw away before giving up on the
 * client. Reading past the limit is what lets the 413 be delivered at all, but
 * it is not open-ended: a client that ignores the response and keeps streaming
 * gets its socket cut once it has spent this much.
 */
export const OVERSIZE_DRAIN_BUDGET_BYTES = 1024 * 1024;

/**
 * How long a session with nothing open may sit before it is reclaimed.
 *
 * Generous on purpose. Reclaiming is spec-sanctioned — the server MAY end a
 * session at any time and MUST then answer 404, and the client MUST start a
 * new one — but the SDK's TypeScript client has no 404 branch, so it will not
 * re-initialize. A client that is alive and merely quiet therefore pays for
 * this, and half an hour keeps that to clients that have genuinely stopped
 * working. Sessions holding an open stream are never reclaimed regardless.
 */
export const DEFAULT_SESSION_IDLE_TIMEOUT_MS = 30 * 60 * 1000;

/** Sweep often enough to bound overshoot, without waking up for no reason. */
function sweepIntervalFor(idleTimeoutMs: number): number {
  return Math.max(25, Math.min(Math.floor(idleTimeoutMs / 4), 60_000));
}

/** Probe idle TCP connections so a vanished peer surfaces as a socket close. */
const SOCKET_KEEPALIVE_DELAY_MS = 60_000;

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
  /**
   * Reclaim a session with nothing open after this long. Exposed for tests,
   * which cannot wait out the default; there is deliberately no CLI flag.
   */
  sessionIdleTimeoutMs?: number;
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

function sendJson(
  res: ServerResponse,
  status: number,
  body: unknown,
  closeConnection = false
): void {
  const payload = JSON.stringify(body);
  const headers: Record<string, string | number> = {
    "Content-Type": "application/json",
    "Content-Length": Buffer.byteLength(payload),
  };
  // Used when the request body was refused part-read: the rest of it is of no
  // interest, and keeping the connection alive would only invite more of it.
  if (closeConnection) headers.Connection = "close";
  res.writeHead(status, headers);
  res.end(payload);
}

function sendJsonRpcError(
  res: ServerResponse,
  status: number,
  code: number,
  message: string,
  closeConnection = false
): void {
  sendJson(
    res,
    status,
    {
      jsonrpc: "2.0",
      error: { code, message },
      id: null,
    },
    closeConnection
  );
}

/**
 * Read a bounded request body. Rejects (rather than buffers) oversized posts.
 *
 * Once the limit is crossed the buffered chunks are dropped, but the stream is
 * left flowing and discarded rather than destroyed. Destroying the request here
 * tears down the socket, and the 413 the caller then writes goes nowhere — the
 * client sees a connection reset instead of the error. Draining costs at most
 * OVERSIZE_DRAIN_BUDGET_BYTES, after which the socket does go.
 */
function readBody(req: IncomingMessage): Promise<Buffer> {
  return new Promise((resolvePromise, rejectPromise) => {
    const chunks: Buffer[] = [];
    let total = 0;
    let overflowed = false;
    req.on("data", (chunk: Buffer) => {
      total += chunk.length;
      if (!overflowed && total > MAX_REQUEST_BODY_BYTES) {
        overflowed = true;
        chunks.length = 0;
        rejectPromise(new Error("Request body too large"));
      }
      if (!overflowed) {
        chunks.push(chunk);
        return;
      }
      if (total > MAX_REQUEST_BODY_BYTES + OVERSIZE_DRAIN_BUDGET_BYTES) {
        req.destroy();
      }
    });
    req.on("end", () => {
      if (!overflowed) resolvePromise(Buffer.concat(chunks));
    });
    req.on("error", (error) => {
      // Already rejected on overflow; a reset while draining is expected.
      if (!overflowed) rejectPromise(error);
    });
  });
}

type SessionEntry = {
  transport: StreamableHTTPServerTransport;
  server: McpServer;
  dispose: () => void;
  closing: boolean;
  /** Last time a request for this session arrived or one of its responses ended. */
  lastActivityAt: number;
  /**
   * Responses still open for this session: in-flight requests and long-lived
   * SSE streams alike. A `Set` rather than a counter so a response closing
   * after the entry was already removed cannot double-count.
   */
  openResponses: Set<ServerResponse>;
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
  const sessionIdleTimeoutMs =
    options.sessionIdleTimeoutMs ?? DEFAULT_SESSION_IDLE_TIMEOUT_MS;
  const log = options.log ?? ((message: string) => console.error(message));

  const sessions = new Map<string, SessionEntry>();
  const sockets = new Set<Socket>();

  /**
   * Record a response against its session. Stamping on arrival is not enough
   * on its own: a session whose only traffic was one hour-long stream would be
   * instantly reapable the moment that stream ended, so the close stamps too.
   */
  function trackResponse(entry: SessionEntry, res: ServerResponse): void {
    entry.lastActivityAt = Date.now();
    entry.openResponses.add(res);
    res.once("close", () => {
      entry.openResponses.delete(res);
      entry.lastActivityAt = Date.now();
    });
  }

  /**
   * Close sessions that have been idle past the timeout.
   *
   * "Idle" means nothing open at all — no in-flight request and no SSE stream.
   * An open stream is treated as proof of life rather than as something to
   * reap: the SDK writes keep-alive frames on it every 15s, so a peer that
   * died takes its stream down with it once those writes go unacknowledged,
   * and a client that is merely quiet keeps the session it is still holding.
   *
   * The gap this leaves, deliberately: a peer alive at TCP but dead at the
   * application layer holds its stream open forever and is never reclaimed.
   * Under pressure the sweep below finds nothing and the 503 stands, which is
   * the honest answer, because such a session looks alive at every layer that
   * can be observed from here.
   */
  function reapIdleSessions(): void {
    const cutoff = Date.now() - sessionIdleTimeoutMs;
    for (const [sessionId, entry] of Array.from(sessions)) {
      if (entry.closing) continue;
      if (entry.openResponses.size > 0) continue;
      if (entry.lastActivityAt > cutoff) continue;
      log(`[Minutes] MCP HTTP session idle, reclaiming: ${sessionId}`);
      void closeSession(sessionId);
    }
  }

  const reaper = setInterval(
    reapIdleSessions,
    sweepIntervalFor(sessionIdleTimeoutMs)
  );
  // Never hold the process open on the reaper's account.
  reaper.unref?.();

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
    // Sweep before refusing, so the limit is measured against sessions that
    // are actually live rather than against ones nobody reclaimed yet. This
    // makes the 503 correct even if the interval timer were somehow starved.
    reapIdleSessions();

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
        const entry: SessionEntry = {
          transport,
          server: instance.server,
          dispose: instance.dispose,
          closing: false,
          lastActivityAt: Date.now(),
          openResponses: new Set(),
        };
        sessions.set(sessionId, entry);
        // Track here, not around the handleRequest below: this fires *during*
        // handleRequest, so there is no entry to attach to any earlier.
        trackResponse(entry, res);
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
        sendJsonRpcError(res, 413, -32000, "Request body too large", true);
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
      trackResponse(entry, res);
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
    // A peer that vanishes without closing its socket (sleep, VPN drop) would
    // otherwise hold its SSE stream, and so its session, until TCP gave up on
    // its own schedule. Keep-alive probes bring that forward.
    socket.setKeepAlive(true, SOCKET_KEEPALIVE_DELAY_MS);
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
      clearInterval(reaper);
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
