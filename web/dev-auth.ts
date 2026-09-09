import type { IncomingMessage, ServerResponse } from "node:http";
import type { Plugin } from "vite";

// Development source files have the same session boundary as embedded assets.
export function oidcDevelopmentGuard(target: string): Plugin {
  async function guard(request: IncomingMessage, response: ServerResponse, next: () => void) {
    // Keep dot segments intact, matching the embedded document router.
    const path = request.url?.split("?")[0] || "/";
    response.setHeader("Cache-Control", "private, no-store");
    if (
      path === "/api" ||
      path.startsWith("/api/") ||
      path.startsWith("/auth/oidc/") ||
      ["/livez", "/readyz"].includes(path)
    )
      return next();
    let status = 401;
    if (!request.headers.authorization) {
      try {
        const session = await fetch(new URL("/api/v1/session", target), {
          headers: { cookie: request.headers.cookie || "" },
          redirect: "error",
          signal: AbortSignal.timeout(5000),
        });
        status = session.status;
        await session.body?.cancel();
      } catch {
        status = 503;
      }
    }
    if (status === 200) return next();
    const document =
      ["/", "/fleets", "/templates", "/templates/new", "/auth", "/changes"].includes(path) ||
      (/^\/fleets\/[a-zA-Z0-9_.-]{1,128}$/.test(path) &&
        !["/fleets/.", "/fleets/.."].includes(path)) ||
      (/^\/templates\/[a-zA-Z0-9_.-]{1,128}\/revisions\/new$/.test(path) &&
        !["/templates/./revisions/new", "/templates/../revisions/new"].includes(path));
    if (status === 401 && request.method === "GET" && document && !request.headers.authorization) {
      response.writeHead(302, {
        location: `/auth/oidc/login?${new URLSearchParams({ return_to: request.url || "/fleets" })}`,
      });
    } else {
      response.writeHead(status === 401 ? 401 : 503, {
        "content-type": "application/problem+json",
        "www-authenticate": "Bearer",
      });
    }
    response.end();
  }
  return {
    name: "shaula-oidc-development-guard",
    configureServer(server) {
      server.middlewares.use((req, res, next) => {
        void guard(req, res, next);
      });
    },
    configurePreviewServer(server) {
      server.middlewares.use((req, res, next) => {
        void guard(req, res, next);
      });
    },
  };
}
