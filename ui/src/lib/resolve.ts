/// Finding the PDS that hosts an account, from whatever the user typed.
///
/// A handle resolves to a DID, the DID document names the PDS, and that URL is
/// what the app then talks to. Resolution itself is asked of the current origin
/// (the gateway is authoritative for its own handle namespace); the DID document
/// comes from the PLC directory or the did:web host, which is the real authority
/// for where the repository lives.

const PLC_DIRECTORY = "https://plc.directory";

export type Resolution = {
  did: string;
  handle: string | null;
  /// Base URL of the PDS hosting this account.
  service: string;
};

export function looksLikeHandle(value: string) {
  const v = value.trim();
  return !v.startsWith("did:") && !v.includes("@") && v.includes(".");
}

export function looksLikeDid(value: string) {
  return value.trim().startsWith("did:");
}

/// True for an input we can resolve: a handle or a DID, but not an email.
export function isResolvable(value: string) {
  return looksLikeDid(value) || looksLikeHandle(value);
}

async function json(input: string, init?: RequestInit): Promise<unknown> {
  const response = await fetch(input, init);
  if (!response.ok) throw new Error(`${input} returned ${response.status}`);
  return response.json();
}

/// Asks `origin` to resolve a handle. The gateway answers for the handles it
/// owns and resolves the rest over the network, so one call covers both.
export async function resolveHandle(
  handle: string,
  origin = window.location.origin,
): Promise<string> {
  const target = new URL("/xrpc/com.atproto.identity.resolveHandle", origin);
  target.searchParams.set("handle", handle);
  const body = (await json(target.toString())) as { did?: string };
  if (!body.did) throw new Error(`no DID for ${handle}`);
  return body.did;
}

type DidDocument = {
  id?: string;
  alsoKnownAs?: string[];
  service?: { id?: string; type?: string; serviceEndpoint?: string }[];
};

export async function fetchDidDocument(did: string): Promise<DidDocument> {
  if (did.startsWith("did:plc:")) {
    return (await json(`${PLC_DIRECTORY}/${did}`)) as DidDocument;
  }
  if (did.startsWith("did:web:")) {
    const host = did.slice("did:web:".length).split(":")[0] ?? "";
    const decoded = host.replace(/%3A/gi, ":");
    const scheme = /^(localhost|127\.0\.0\.1)/.test(decoded) ? "http" : "https";
    return (await json(
      `${scheme}://${decoded}/.well-known/did.json`,
    )) as DidDocument;
  }
  throw new Error(`unsupported DID method in ${did}`);
}

export function pdsEndpoint(doc: DidDocument): string | null {
  const services = doc.service ?? [];
  const found =
    services.find((s) => s.id?.endsWith("#atproto_pds")) ??
    services.find((s) => s.type === "AtprotoPersonalDataServer");
  const endpoint = found?.serviceEndpoint;
  return endpoint ? endpoint.replace(/\/+$/, "") : null;
}

export function claimedHandle(doc: DidDocument): string | null {
  const aka = (doc.alsoKnownAs ?? []).find((a) => a.startsWith("at://"));
  return aka ? aka.slice("at://".length) : null;
}

/// Resolves a handle or DID all the way to the PDS that hosts it.
export async function detectPds(
  identifier: string,
  origin = window.location.origin,
): Promise<Resolution> {
  const value = identifier.trim();
  if (!isResolvable(value)) {
    throw new Error("Enter a handle or a DID to detect the server.");
  }

  const did = looksLikeDid(value) ? value : await resolveHandle(value, origin);
  const doc = await fetchDidDocument(did);
  const service = pdsEndpoint(doc);

  if (!service) {
    throw new Error(`${did} publishes no PDS endpoint.`);
  }

  return { did, handle: claimedHandle(doc) ?? (looksLikeDid(value) ? null : value), service };
}
