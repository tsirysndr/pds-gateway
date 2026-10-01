import { describe, expect, it } from "vitest";
import { detectPds, isResolvable, looksLikeHandle, pdsEndpoint } from "./resolve";
import { preferOrigin } from "./servers";
import { ALICE, BOB, ORIGIN } from "../mocks/fixtures";

describe("identifying what the user typed", () => {
  it("accepts handles and DIDs but not emails", () => {
    expect(looksLikeHandle("alice.rocksky.social")).toBe(true);
    expect(looksLikeHandle("alice@example.com")).toBe(false);
    expect(looksLikeHandle("did:plc:abc")).toBe(false);

    expect(isResolvable("alice.rocksky.social")).toBe(true);
    expect(isResolvable("did:plc:abc")).toBe(true);
    // An email cannot be resolved to a server, which is why sign-in asks for a
    // handle.
    expect(isResolvable("alice@example.com")).toBe(false);
  });
});

describe("reading the PDS out of a DID document", () => {
  it("prefers the atproto_pds service and trims the trailing slash", () => {
    expect(
      pdsEndpoint({
        service: [
          { id: "#other", type: "Other", serviceEndpoint: "https://nope.example" },
          {
            id: "#atproto_pds",
            type: "AtprotoPersonalDataServer",
            serviceEndpoint: "https://radxa.rocksky.social/",
          },
        ],
      }),
    ).toBe("https://radxa.rocksky.social");
  });

  it("returns null when the document names no PDS", () => {
    expect(pdsEndpoint({ service: [] })).toBeNull();
    expect(pdsEndpoint({})).toBeNull();
  });
});

describe("detecting the server from a handle", () => {
  it("finds the node that hosts a remote account", async () => {
    const result = await detectPds(BOB.handle, ORIGIN);
    expect(result.did).toBe(BOB.did);
    expect(result.handle).toBe(BOB.handle);
    expect(result.service).toBe(BOB.service);
  });

  it("works from a DID as well as a handle", async () => {
    const result = await detectPds(BOB.did, ORIGIN);
    expect(result.service).toBe(BOB.service);
  });

  it("refuses an email, rather than guessing a server", async () => {
    await expect(detectPds("alice@example.com", ORIGIN)).rejects.toThrow(
      /handle or a DID/,
    );
  });

  it("reports an unclaimed handle", async () => {
    await expect(detectPds("nobody.rocksky.social", ORIGIN)).rejects.toThrow();
  });

  it("keeps the page origin when the account lives on this server", async () => {
    const result = await detectPds(ALICE.handle, ORIGIN);
    // alice is hosted by whatever serves this page, so requests stay on the
    // origin rather than being re-pointed at an equivalent URL.
    expect(preferOrigin(result.service, ORIGIN)).toBe(ORIGIN);
  });
});
