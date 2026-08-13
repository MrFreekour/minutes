import { describe, expect, it } from "vitest";

import {
  HTTP_BIND_HOST,
  DEFAULT_HTTP_PORT,
  isAllowedContentType,
  isAllowedHostHeader,
  isAllowedOrigin,
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
