import { describe, expect, it } from "vitest";
import { fromBase64Url, toBase64Url, toPublicKey, toPublicKeyRequest } from "./security";

const CHALLENGE = "Y2hhbGxlbmdl";

describe("reading WebAuthn options from a server", () => {
  const assertion = { challenge: CHALLENGE, userVerification: "required" as const };

  it("accepts the options on their own", () => {
    const result = toPublicKeyRequest(assertion);
    expect(result.challenge).toBeInstanceOf(Uint8Array);
    expect(result.userVerification).toBe("required");
  });

  it("accepts them wrapped in publicKey", () => {
    // WebAuthn's own JSON helpers return `{publicKey: {...}}`, and an
    // implementation that forwards that verbatim nests it one level deeper.
    // That used to crash sign-in on "cannot read properties of undefined".
    const result = toPublicKeyRequest({ publicKey: assertion } as never);
    expect(result.challenge).toBeInstanceOf(Uint8Array);
    expect(result.userVerification).toBe("required");
  });

  it("unwraps creation options too", () => {
    const creation = {
      challenge: CHALLENGE,
      rp: { name: "rocksky.social" },
      user: { id: CHALLENGE, name: "alice", displayName: "alice" },
      pubKeyCredParams: [{ type: "public-key" as const, alg: -7 }],
    };
    const result = toPublicKey({ publicKey: creation } as never);
    expect(result.challenge).toBeInstanceOf(Uint8Array);
    expect(result.user.id).toBeInstanceOf(Uint8Array);
  });

  it("says which field is missing rather than failing on undefined", () => {
    expect(() => fromBase64Url(undefined as never)).toThrow(/missing a required field/);
    expect(() => fromBase64Url("")).toThrow(/missing a required field/);
    expect(() => toPublicKeyRequest({} as never)).toThrow(/missing a required field/);
  });
});

describe("encoding a credential for the server", () => {
  it("encodes rawId exactly as the browser spells credential.id", () => {
    // `credential.id` is unpadded base64url, and at least one implementation
    // requires `id` and `rawId` to be the same string. Padding or the standard
    // alphabet here would have every registration refused.
    const bytes = new Uint8Array([251, 255, 190, 0, 1, 2, 3]);
    const encoded = toBase64Url(bytes.buffer);

    expect(encoded).not.toContain("=");
    expect(encoded).not.toContain("+");
    expect(encoded).not.toContain("/");
    expect(Array.from(fromBase64Url(encoded))).toEqual(Array.from(bytes));
  });
});
