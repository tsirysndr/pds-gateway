import { http, HttpResponse } from "msw";
import { ALICE, BOB, didDocument, PLC, session } from "./fixtures";

const ACCOUNTS = [ALICE, BOB];

export const handlers = [
  // A browser preflights cross-origin requests, so the mocks answer those too;
  // without this, strict mode reports every preflight as unhandled.
  http.options("*", () =>
    new HttpResponse(null, {
      status: 204,
      headers: {
        "access-control-allow-origin": "*",
        "access-control-allow-methods": "GET,POST,OPTIONS",
        "access-control-allow-headers": "*",
      },
    }),
  ),

  // Handle resolution, as the gateway answers it for its whole namespace.
  http.get("*/xrpc/com.atproto.identity.resolveHandle", ({ request }) => {
    const handle = new URL(request.url).searchParams.get("handle");
    const found = ACCOUNTS.find((a) => a.handle === handle);
    return found
      ? HttpResponse.json({ did: found.did })
      : HttpResponse.json(
          { error: "UnableToResolveHandle", message: "Unable to resolve handle." },
          { status: 400 },
        );
  }),

  http.get(`${PLC}/:did`, ({ params }) => {
    const found = ACCOUNTS.find((a) => a.did === params.did);
    return found
      ? HttpResponse.json(didDocument(found))
      : new HttpResponse(null, { status: 404 });
  }),

  http.get("*/_gateway/pds", () =>
    HttpResponse.json({
      servers: [
        { name: "local", url: "http://localhost:3000" },
        { name: "radxa", url: "https://radxa.rocksky.social" },
      ],
    }),
  ),

  http.get("*/xrpc/com.atproto.server.describeServer", () =>
    HttpResponse.json({
      did: "did:web:rocksky.social",
      availableUserDomains: [".rocksky.social"],
      inviteCodeRequired: false,
      blobUploadLimit: 5242880,
    }),
  ),

  http.post("*/xrpc/com.atproto.server.createSession", async ({ request }) => {
    const body = (await request.json()) as { identifier: string; password: string };
    const found = ACCOUNTS.find((a) => a.handle === body.identifier);
    if (!found) {
      return HttpResponse.json({ error: "AccountNotFound" }, { status: 401 });
    }
    if (body.password !== "correct-horse") {
      return HttpResponse.json(
        { error: "AuthenticationFailed", message: "Invalid identifier or password" },
        { status: 401 },
      );
    }
    return HttpResponse.json(session(found));
  }),

  http.get("*/xrpc/com.atproto.server.getSession", () =>
    HttpResponse.json(session(ALICE)),
  ),

  http.get("*/xrpc/com.atproto.server.listAppPasswords", () =>
    HttpResponse.json({
      passwords: [{ name: "my phone", createdAt: "2026-01-01T00:00:00Z" }],
    }),
  ),

  http.get("*/account/security", () => HttpResponse.json({ state: "disabled" })),
];
