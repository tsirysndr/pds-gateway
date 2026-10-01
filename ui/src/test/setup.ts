import "@testing-library/jest-dom/vitest";
import { afterAll, afterEach, beforeAll } from "vitest";
import { cleanup } from "@testing-library/react";
import { server } from "../mocks/server";

// Unhandled requests are an error: a test that silently hits the network is a
// test that passes for the wrong reason.
// msw v3 calls these "frames" (requests and websocket connections alike).
beforeAll(() => server.listen({ onUnhandledFrame: "error" }));
afterEach(() => {
  cleanup();
  server.resetHandlers();
  window.localStorage.clear();
});
afterAll(() => server.close());
