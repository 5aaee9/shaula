let csrf: string | null = null;
let expired = false;
const listeners = new Set<() => void>();

export function subscribeAuthentication(listener: () => void) {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}
export const authenticationExpired = () => expired;
export const loginUrl = () =>
  `/auth/oidc/login?${new URLSearchParams({ return_to: location.pathname + location.search })}`;

export function clearAuthentication() {
  csrf = null;
  if (expired) return;
  expired = true;
  for (const listener of listeners) listener();
}

export async function authenticatedFetch(path: string, options: RequestInit = {}) {
  if (expired) throw new Error("Authentication required.");
  const headers = new Headers(options.headers);
  if (!["GET", "HEAD", "OPTIONS"].includes((options.method || "GET").toUpperCase())) {
    if (!csrf) throw new Error("Session unavailable. Sign in again before making changes.");
    headers.set("x-csrf-token", csrf);
  }
  const response = await fetch(path, {
    ...options,
    headers,
    credentials: "same-origin",
    cache: "no-store",
    redirect: "error",
  });
  if (response.status === 401) clearAuthentication();
  if (!expired && path === "/api/v1/session" && response.ok)
    csrf = response.headers.get("x-csrf-token");
  return response;
}

export async function logout() {
  const response = await authenticatedFetch("/auth/oidc/logout", { method: "POST" });
  if (!response.ok && response.status !== 401)
    throw new Error("Sign out failed. Please try again.");
  clearAuthentication();
}
