import { describe, expect, it } from "vitest";
import { fromBase64Url, toPublicKey, toPublicKeyRequest } from "./security";

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
