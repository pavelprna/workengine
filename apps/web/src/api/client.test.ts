import { expect, test } from "bun:test";

test("local API is same-origin", () => {
  expect(new URL("/api/v0/health", "http://localhost").pathname).toBe(
    "/api/v0/health",
  );
});
