export const PLC = "https://plc.directory";
export const ORIGIN = "http://localhost:3000";
export const RADXA = "https://radxa.rocksky.social";

export const ALICE = {
  did: "did:plc:alice234567alice234567al",
  handle: "alice.rocksky.social",
  service: ORIGIN,
};

export const BOB = {
  did: "did:plc:bob234567bob234567bob234",
  handle: "bob.rocksky.social",
  service: RADXA,
};

/// An account protected by an authenticator, so the plain sign-in path stays
/// testable on its own.
export const CARA = {
  did: "did:plc:cara234567cara234567car",
  handle: "cara.rocksky.social",
  service: RADXA,
};

export const TOTP_ACCOUNT = CARA.handle;

export function didDocument(account: { did: string; handle: string; service: string }) {
  return {
    id: account.did,
    alsoKnownAs: [`at://${account.handle}`],
    service: [
      {
        id: "#atproto_pds",
        type: "AtprotoPersonalDataServer",
        serviceEndpoint: account.service,
      },
    ],
  };
}

export function session(account: { did: string; handle: string }) {
  return {
    did: account.did,
    handle: account.handle,
    email: "alice@example.com",
    emailConfirmed: true,
    accessJwt: "access-token",
    refreshJwt: "refresh-token",
  };
}
