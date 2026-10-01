/// A minimal XRPC client pointed at one PDS.
///
/// Every call goes to the base URL the app has resolved for the account, so once
/// a handle is resolved the browser talks to that PDS directly rather than
/// through whatever host served this page.

export class XrpcError extends Error {
  constructor(
    readonly status: number,
    readonly error: string,
    message: string,
  ) {
    super(message);
    this.name = "XrpcError";
  }

  /// The access token is gone or rejected, so the session should be dropped.
  get isAuthExpired() {
    return (
      this.status === 400 &&
      ["ExpiredToken", "InvalidToken"].includes(this.error)
    );
  }
}

export type Auth = { accessJwt: string; refreshJwt: string } | null;

function url(base: string, nsid: string, params?: Record<string, unknown>) {
  const target = new URL(`/xrpc/${nsid}`, base);
  for (const [key, value] of Object.entries(params ?? {})) {
    if (value === undefined || value === null || value === "") continue;
    // Repeated parameters (such as `dids`) arrive as arrays.
    if (Array.isArray(value)) {
      for (const item of value) target.searchParams.append(key, String(item));
    } else {
      target.searchParams.set(key, String(value));
    }
  }
  return target.toString();
}

async function parse(response: Response) {
  const text = await response.text();
  const body = text ? safeJson(text) : null;

  if (!response.ok) {
    const error =
      (body as { error?: string } | null)?.error ?? `HTTP${response.status}`;
    const message =
      (body as { message?: string } | null)?.message ?? response.statusText;
    throw new XrpcError(response.status, error, message);
  }
  return body;
}

function safeJson(text: string): unknown {
  try {
    return JSON.parse(text);
  } catch {
    return { raw: text };
  }
}

export type Client = {
  base: string;
  get<T>(nsid: string, params?: Record<string, unknown>): Promise<T>;
  post<T>(nsid: string, body?: unknown, params?: Record<string, unknown>): Promise<T>;
  upload<T>(nsid: string, file: Blob): Promise<T>;
};

export function createClient(base: string, token?: string): Client {
  const headers = () => {
    const out: Record<string, string> = {};
    if (token) out.authorization = `Bearer ${token}`;
    return out;
  };

  return {
    base,
    async get<T>(nsid: string, params?: Record<string, unknown>) {
      const response = await fetch(url(base, nsid, params), {
        headers: headers(),
      });
      return (await parse(response)) as T;
    },
    async post<T>(
      nsid: string,
      body?: unknown,
      params?: Record<string, unknown>,
    ) {
      const response = await fetch(url(base, nsid, params), {
        method: "POST",
        headers: {
          ...headers(),
          ...(body === undefined ? {} : { "content-type": "application/json" }),
        },
        body: body === undefined ? undefined : JSON.stringify(body),
      });
      return (await parse(response)) as T;
    },
    async upload<T>(nsid: string, file: Blob) {
      const response = await fetch(url(base, nsid), {
        method: "POST",
        headers: {
          ...headers(),
          "content-type": file.type || "application/octet-stream",
        },
        body: file,
      });
      return (await parse(response)) as T;
    },
  };
}
