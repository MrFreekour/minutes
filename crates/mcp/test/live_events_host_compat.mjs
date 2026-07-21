#!/usr/bin/env node
import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { homedir, tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { StdioClientTransport } from "@modelcontextprotocol/sdk/client/stdio.js";
import { ResourceUpdatedNotificationSchema } from "@modelcontextprotocol/sdk/types.js";

const __dirname = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(__dirname, "../../..");
const defaultServer = {
  command: "node",
  args: [join(repoRoot, "crates", "mcp", "dist", "index.js")],
};
const LIVE_URI = "minutes://events/live";

function loadClaudeDesktopMinutesConfig() {
  const path = join(homedir(), "Library", "Application Support", "Claude", "claude_desktop_config.json");
  if (!existsSync(path)) {
    return { status: "skipped", reason: `${path} does not exist` };
  }
  const config = JSON.parse(readFileSync(path, "utf8"));
  const server = config?.mcpServers?.minutes;
  if (!server?.command) {
    return { status: "skipped", reason: "Claude Desktop config has no mcpServers.minutes command" };
  }
  return { status: "ready", command: server.command, args: server.args ?? [] };
}

function loadCodexMinutesConfig() {
  const path = join(homedir(), ".codex", "config.toml");
  if (!existsSync(path)) {
    return { status: "skipped", reason: `${path} does not exist` };
  }
  const raw = readFileSync(path, "utf8");
  const section = raw.match(/\[mcp_servers\.minutes\]([\s\S]*?)(?=\n\[|$)/);
  if (!section) {
    return { status: "skipped", reason: "Codex config has no [mcp_servers.minutes] section" };
  }
  const body = section[1];
  const command = body.match(/command\s*=\s*"([^"]+)"/)?.[1];
  const argsRaw = body.match(/args\s*=\s*\[([\s\S]*?)\]/)?.[1] ?? "";
  const args = [...argsRaw.matchAll(/"([^"]*)"/g)].map((match) => match[1]);
  if (!command) {
    return { status: "skipped", reason: "Codex minutes server has no command" };
  }
  return { status: "ready", command, args };
}

const HOSTS = [
  ["candidate-stdio", () => ({ status: "ready", ...defaultServer })],
  ["claude-desktop-config", loadClaudeDesktopMinutesConfig],
  ["codex-cli-config", loadCodexMinutesConfig],
];

function normalizeServerConfig(raw) {
  if (raw.status !== "ready") return raw;
  return {
    ...raw,
    command: raw.command ?? defaultServer.command,
    args: raw.args?.length ? raw.args : defaultServer.args,
  };
}

function appendCompatEvent(home, seq, body) {
  const event = {
    v: 1,
    seq,
    timestamp: new Date().toISOString(),
    event_type: "sensitive.marker",
    session_id: "HOST_COMPAT_PRIVATE_SESSION",
    text: body,
  };
  const minutesDir = join(home, ".minutes");
  writeFileSync(join(minutesDir, "events.jsonl"), `${JSON.stringify(event)}\n`, { flag: "a" });
  writeFileSync(join(minutesDir, "events.seq"), `${seq}\n`);
  return event;
}

async function runHostSmoke(host, serverConfig) {
  const tempHome = mkdtempSync(join(tmpdir(), `minutes-${host}-`));
  const minutesDir = join(tempHome, ".minutes");
  mkdirSync(minutesDir, { recursive: true });
  writeFileSync(join(minutesDir, "agents.allow"), "compat-agent: agent.annotation\n");

  const notifications = [];
  const stderrChunks = [];
  const transport = new StdioClientTransport({
    command: serverConfig.command,
    args: serverConfig.args,
    cwd: repoRoot,
    env: {
      ...process.env,
      HOME: tempHome,
      USERPROFILE: tempHome,
      MINUTES_MCP_EVENT_POLL_MS: "50",
      RUST_LOG: "info",
    },
    stderr: "pipe",
  });
  transport.stderr?.on("data", (chunk) => stderrChunks.push(String(chunk)));

  const client = new Client(
    { name: `minutes-${host}-compat`, version: "0.0.0" },
    { capabilities: {} }
  );
  client.setNotificationHandler(ResourceUpdatedNotificationSchema, (notification) => {
    notifications.push(notification.params.uri);
  });

  try {
    await client.connect(transport);
    let subscriptionRejected = false;
    try {
      await client.subscribeResource({ uri: LIVE_URI });
    } catch {
      subscriptionRejected = true;
    }
    if (!subscriptionRejected) {
      throw new Error("restricted event subscriptions were unexpectedly enabled");
    }
    appendCompatEvent(tempHome, 1, `PRIVATE_HOST_COMPAT_CANARY_${host}`);
    await new Promise((resolve) => setTimeout(resolve, 250));
    if (notifications.length !== 0) {
      throw new Error("a hidden event produced a resource notification");
    }

    const read = await client.readResource({
      uri: `${LIVE_URI}?since_seq=0&limit=10`,
    });
    const text = read.contents?.[0]?.text ?? "";
    const payload = JSON.parse(text);
    if (
      payload.latest_seq !== 0 ||
      payload.reconnect?.cursor !== 0 ||
      !Array.isArray(payload.events) ||
      payload.events.length !== 0 ||
      !String(payload.unavailable ?? "").includes("non-sensitive cursor") ||
      text.includes("PRIVATE_HOST_COMPAT_CANARY") ||
      text.includes("sensitive.marker")
    ) {
      throw new Error("hidden activity changed the constant unavailable resource");
    }

    return {
      host,
      status: "passed",
      command: serverConfig.command,
      args: serverConfig.args,
      subscription: "rejected",
      notifications: notifications.length,
      read_uri: `${LIVE_URI}?since_seq=0&limit=10`,
      reconnect_cursor: payload.reconnect.cursor,
    };
  } catch (error) {
    return {
      host,
      status: "failed",
      command: serverConfig.command,
      args: serverConfig.args,
      error: error instanceof Error ? error.message : String(error),
      stderr: stderrChunks.join("").slice(-2000),
    };
  } finally {
    await client.close().catch(() => {});
    await transport.close().catch(() => {});
    rmSync(tempHome, { recursive: true, force: true });
  }
}

const results = [];
for (const [host, loader] of HOSTS) {
  const config = normalizeServerConfig(loader());
  if (config.status !== "ready") {
    results.push({ host, ...config });
    continue;
  }
  const configuredEntry = config.args.find((arg) => arg.endsWith("index.js"));
  if (
    host !== "candidate-stdio" &&
    (!configuredEntry ||
      resolve(configuredEntry) !== resolve(defaultServer.args[0]))
  ) {
    results.push({
      host,
      status: "skipped",
      reason: "configured server is outside this candidate checkout",
    });
    continue;
  }
  results.push(await runHostSmoke(host, config));
}

console.log(JSON.stringify({ checked_at: new Date().toISOString(), results }, null, 2));

const failed = results.filter((result) => result.status === "failed");
if (failed.length > 0) {
  process.exitCode = 1;
}
