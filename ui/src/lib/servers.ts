import type { ServerDescription } from "./types";

export type KnownServer = { name: string; url: string; handleDomains?: string[] };

/// The servers the dropdown offers.
///
/// The gateway publishes its fleet so the list is not hardcoded here. Each entry
/// is a public URL: the gateway's own origin stands for the PDS behind it, so
/// choosing it sends requests through the gateway, which forwards them to that
/// node — never to an address only the server can reach.
export async function fetchKnownServers(
  origin = window.location.origin,
): Promise<KnownServer[]> {
  const fallback: KnownServer[] = [{ name: "this server", url: origin }];
  try {
    const response = await fetch(new URL("/_gateway/pds", origin));
    if (!response.ok) return fallback;
    const body = (await response.json()) as { servers?: KnownServer[] };
    const servers = (body.servers ?? []).filter((s) => s.url);
    return servers.length > 0 ? servers : fallback;
  } catch {
    return fallback;
  }
}

export async function describeServer(base: string): Promise<ServerDescription> {
  const response = await fetch(
    new URL("/xrpc/com.atproto.server.describeServer", base),
  );
  if (!response.ok) throw new Error(`describeServer returned ${response.status}`);
  return (await response.json()) as ServerDescription;
}

export function hostOf(url: string) {
  try {
    return new URL(url).host;
  } catch {
    return url;
  }
}

/// Keeps the page's own origin when a resolved endpoint points back at it, so a
/// request is not sent to a second, equivalent URL for the same host.
export function preferOrigin(service: string, origin = window.location.origin) {
  return hostOf(service) === hostOf(origin) ? origin : service;
}
