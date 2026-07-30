import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    root: ".",
    include: ["src/**/*.test.ts", "ui/src/**/*.test.ts"],
    testTimeout: 15_000,
  },
});
