export class ApiError extends Error {
  constructor(
    public status: number,
    public code: string,
    message: string,
  ) {
    super(message);
  }
}

export interface Resource<T> {
  data: T;
  etag: string | null;
}

export async function api<T>(path: string, options: RequestInit = {}): Promise<Resource<T>> {
  const headers = new Headers(options.headers);
  headers.set("accept", "application/json");
  if (options.body && typeof options.body === "string")
    headers.set("content-type", "application/json");
  const response = await fetch(`/api/v1${path}`, {
    ...options,
    headers,
    credentials: "same-origin",
    cache: "no-store",
    redirect: "error",
  });
  const contentType = response.headers.get("content-type") || "";
  if (!contentType.includes("application/json")) {
    throw new ApiError(
      response.status,
      "InvalidResponse",
      "The server did not return a valid API response.",
    );
  }
  const data = await response.json();
  if (!response.ok)
    throw new ApiError(
      response.status,
      data.code || "RequestFailed",
      data.detail || `Request failed (${response.status}).`,
    );
  return { data, etag: response.headers.get("etag") };
}

export const resourcePath = (kind: string, key: string) => `/${kind}/${encodeURIComponent(key)}`;

// Keep the key for an unchanged request so retrying an uncertain write is safe.
export class MutationAttempt {
  private fingerprint = "";
  private key = "";
  headers(
    method: string,
    path: string,
    body: string | undefined,
    etag: string | null,
    create: boolean,
  ) {
    const fingerprint = JSON.stringify([method, path, body, etag, create]);
    if (fingerprint !== this.fingerprint) {
      this.fingerprint = fingerprint;
      this.key = crypto.randomUUID();
    }
    if (!create && !etag)
      throw new Error("The resource version is missing. Refresh before making changes.");
    return {
      "idempotency-key": this.key,
      ...(create ? { "if-none-match": "*" } : { "if-match": etag! }),
    };
  }
}

export function errorMessage(error: unknown): string {
  if (error instanceof ApiError) {
    if (error.status === 401)
      return "Authentication required. Your session is missing or has expired.";
    if (error.status === 403) return "You do not have permission for this operation.";
    if (error.status === 412)
      return "This resource changed since you opened it. Close this dialog, refresh, and try again.";
  }
  return error instanceof Error ? error.message : "The request could not be completed.";
}
