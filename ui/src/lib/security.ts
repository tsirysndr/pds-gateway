/// Two-factor and passkeys over `social.rocksky.auth.*`.
///
/// These are XRPC methods, so the bearer token that authorises every other call
/// authorises these too, and the gateway routes them to the account's own PDS
/// like anything else. A server that does not implement them answers
/// `MethodNotImplemented`, which is how `unsupported` is detected rather than
/// guessing from a 404 on some path.

import { XrpcError, type Client } from "./xrpc";
import type { Session } from "./types";

export type TotpState = "disabled" | "pending" | "enabled" | "unsupported";

export type TotpStatus = {
  state: TotpState;
  recoveryRemaining?: number;
};

export type Enrollment = {
  state: "pending";
  /// `otpauth://` URI for the authenticator QR code.
  uri: string;
  /// The same shared secret, for entering by hand.
  secret: string;
};

export type Passkey = {
  id: string;
  name?: string;
  createdAt?: string;
  lastUsedAt?: string;
};

/// True when the server does not implement the method at all, as opposed to
/// refusing this particular call.
export function isUnsupported(error: unknown) {
  return (
    error instanceof XrpcError &&
    (error.error === "MethodNotImplemented" ||
      error.error === "NotSupported" ||
      error.status === 501 ||
      error.status === 404)
  );
}

export async function fetchTotpStatus(client: Client): Promise<TotpStatus> {
  try {
    return await client.get<TotpStatus>("social.rocksky.auth.getTwoFactor");
  } catch (error) {
    if (isUnsupported(error)) return { state: "unsupported" };
    throw error;
  }
}

export function beginTotp(client: Client, password: string) {
  return client.post<Enrollment>("social.rocksky.auth.beginTwoFactor", { password });
}

export function confirmTotp(client: Client, code: string) {
  return client.post<{ state: "enabled"; recoveryCodes: string[] }>(
    "social.rocksky.auth.confirmTwoFactor",
    { code },
  );
}

export function disableTotp(client: Client, password: string, code: string) {
  return client.post<{ state: "disabled" }>("social.rocksky.auth.disableTwoFactor", {
    password,
    code,
  });
}

export function regenerateRecovery(client: Client, password: string, code: string) {
  return client.post<{ recoveryCodes: string[] }>(
    "social.rocksky.auth.regenerateRecoveryCodes",
    { password, code },
  );
}

export async function listPasskeys(client: Client): Promise<Passkey[]> {
  const body = await client.get<{ passkeys: Passkey[] }>(
    "social.rocksky.auth.listPasskeys",
  );
  return body.passkeys ?? [];
}

/// Registration options as the server sends them, binary fields base64url.
export type CreationOptions = {
  challenge: string;
  rp: { id?: string; name: string };
  user: { id: string; name: string; displayName: string };
  pubKeyCredParams: { type: "public-key"; alg: number }[];
  timeout?: number;
  attestation?: AttestationConveyancePreference;
  excludeCredentials?: { id: string; type: "public-key" }[];
  authenticatorSelection?: AuthenticatorSelectionCriteria;
};

export function beginPasskeyRegistration(
  client: Client,
  input: { password: string; code?: string; name?: string },
) {
  return client.post<{ requestId: string; publicKey: CreationOptions }>(
    "social.rocksky.auth.beginPasskeyRegistration",
    input,
  );
}

export function finishPasskeyRegistration(
  client: Client,
  input: { requestId: string; credential: unknown },
) {
  return client.post<{ passkey: Passkey }>(
    "social.rocksky.auth.finishPasskeyRegistration",
    input,
  );
}

export function deletePasskey(client: Client, id: string, password: string) {
  return client.post("social.rocksky.auth.deletePasskey", { id, password });
}

// --- signing in with a passkey -------------------------------------------
// These take no token: they are how a session begins, so they use a client
// built for the resolved PDS without one.

export function beginPasskeyLogin(client: Client, identifier: string) {
  return client.post<{ requestId: string; publicKey: RequestOptions }>(
    "social.rocksky.auth.beginPasskeyLogin",
    { identifier },
  );
}

export function finishPasskeyLogin(
  client: Client,
  input: {
    requestId: string;
    credential: unknown;
    totpCode?: string;
    authFactorToken?: string;
  },
) {
  return client.post<Session>("social.rocksky.auth.finishPasskeyLogin", input);
}

/// Assertion options as the server sends them, binary fields base64url.
export type RequestOptions = {
  challenge: string;
  rpId?: string;
  timeout?: number;
  userVerification?: UserVerificationRequirement;
  allowCredentials?: { id: string; type: "public-key" }[];
};

export function toPublicKeyRequest(
  options: RequestOptions,
): PublicKeyCredentialRequestOptions {
  return {
    ...options,
    challenge: fromBase64Url(options.challenge) as BufferSource,
    allowCredentials: options.allowCredentials?.map((c) => ({
      ...c,
      id: fromBase64Url(c.id) as BufferSource,
    })),
  };
}

/// The shape `finishPasskeyLogin` expects back.
export function assertionJson(credential: PublicKeyCredential) {
  const assertion = credential.response as AuthenticatorAssertionResponse;
  return {
    id: credential.id,
    rawId: toBase64Url(credential.rawId),
    type: credential.type,
    response: {
      clientDataJSON: toBase64Url(assertion.clientDataJSON),
      authenticatorData: toBase64Url(assertion.authenticatorData),
      signature: toBase64Url(assertion.signature),
      userHandle: assertion.userHandle ? toBase64Url(assertion.userHandle) : null,
    },
  };
}

export function passkeysAvailable() {
  return (
    typeof window !== "undefined" &&
    typeof window.PublicKeyCredential !== "undefined" &&
    Boolean(navigator.credentials)
  );
}

// --- WebAuthn JSON <-> browser API ---------------------------------------
// The browser wants ArrayBuffers where the wire format uses base64url.

export function fromBase64Url(value: string): Uint8Array {
  const padded = value.replace(/-/g, "+").replace(/_/g, "/");
  const raw = atob(padded + "=".repeat((4 - (padded.length % 4)) % 4));
  return Uint8Array.from(raw, (c) => c.charCodeAt(0));
}

export function toBase64Url(buffer: ArrayBuffer): string {
  const bytes = new Uint8Array(buffer);
  let raw = "";
  for (const byte of bytes) raw += String.fromCharCode(byte);
  return btoa(raw).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

export function toPublicKey(
  options: CreationOptions,
): PublicKeyCredentialCreationOptions {
  return {
    ...options,
    challenge: fromBase64Url(options.challenge) as BufferSource,
    user: { ...options.user, id: fromBase64Url(options.user.id) as BufferSource },
    excludeCredentials: options.excludeCredentials?.map((c) => ({
      ...c,
      id: fromBase64Url(c.id) as BufferSource,
    })),
  };
}

/// The shape `finishPasskeyRegistration` expects back.
export function credentialJson(credential: PublicKeyCredential) {
  const attestation = credential.response as AuthenticatorAttestationResponse;
  return {
    id: credential.id,
    rawId: toBase64Url(credential.rawId),
    type: credential.type,
    response: {
      clientDataJSON: toBase64Url(attestation.clientDataJSON),
      attestationObject: toBase64Url(attestation.attestationObject),
    },
  };
}
