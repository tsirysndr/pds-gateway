/// Two-factor and passkey management.
///
/// These are not XRPC methods: the atproto lexicon has none, so each PDS exposes
/// its own. The paths here are the ones atoll serves, form-encoded and
/// authorised by its browser session, which is why these calls send credentials
/// and why a server without them reports `unsupported` rather than failing in a
/// way that looks like a bug.

export type TotpState = "disabled" | "pending" | "enabled" | "unsupported";

export type TotpStatus = {
  state: TotpState;
  /// `otpauth://` URI for the authenticator QR code, while enrolling.
  uri?: string;
  /// The same shared secret, for entering by hand.
  secret?: string;
  recoveryCodes?: string[];
};

export class UnsupportedError extends Error {
  constructor() {
    super("This server does not expose two-factor settings.");
    this.name = "UnsupportedError";
  }
}

async function form(
  base: string,
  path: string,
  fields: Record<string, string>,
): Promise<unknown> {
  const body = new URLSearchParams(fields).toString();
  const response = await fetch(new URL(path, base), {
    method: "POST",
    credentials: "include",
    headers: { "content-type": "application/x-www-form-urlencoded" },
    body,
  });

  if (response.status === 404) throw new UnsupportedError();
  const text = await response.text();
  const parsed = text ? safeJson(text) : null;

  if (!response.ok) {
    const message =
      (parsed as { message?: string; error?: string } | null)?.message ??
      (parsed as { error?: string } | null)?.error ??
      `${path} returned ${response.status}`;
    throw new Error(message);
  }
  return parsed;
}

function safeJson(text: string): unknown {
  try {
    return JSON.parse(text);
  } catch {
    return null;
  }
}

export async function fetchTotpStatus(base: string): Promise<TotpStatus> {
  const response = await fetch(new URL("/account/security", base), {
    credentials: "include",
    headers: { accept: "application/json" },
  });
  if (!response.ok) return { state: "unsupported" };

  const body = safeJson(await response.text()) as Partial<TotpStatus> | null;
  if (!body?.state) return { state: "unsupported" };
  return body as TotpStatus;
}

/// Starts enrolment. Returns the shared secret and the URI to render as a QR.
export function beginTotp(base: string, password: string) {
  return form(base, "/account/security/begin", { password }) as Promise<TotpStatus>;
}

export function confirmTotp(base: string, code: string) {
  return form(base, "/account/security/confirm", { code }) as Promise<TotpStatus>;
}

export function disableTotp(base: string, password: string, code: string) {
  return form(base, "/account/security/disable", {
    password,
    code,
  }) as Promise<TotpStatus>;
}

export function regenerateRecovery(base: string, password: string, code: string) {
  return form(base, "/account/security/recovery", {
    password,
    code,
  }) as Promise<TotpStatus>;
}

// --- passkeys -------------------------------------------------------------

export type Passkey = { id: string; name?: string; createdAt?: string };

export async function listPasskeys(base: string): Promise<Passkey[]> {
  const response = await fetch(new URL("/account/passkeys", base), {
    credentials: "include",
    headers: { accept: "application/json" },
  });
  if (!response.ok) throw new UnsupportedError();
  const body = safeJson(await response.text()) as { passkeys?: Passkey[] } | null;
  return body?.passkeys ?? [];
}

export function passkeysAvailable() {
  return (
    typeof window !== "undefined" &&
    typeof window.PublicKeyCredential !== "undefined" &&
    Boolean(navigator.credentials)
  );
}

/// base64url is what WebAuthn JSON uses; the browser API wants ArrayBuffers.
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
