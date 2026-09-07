import { fileURLToPath, URL } from "node:url";
import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import { defineConfig, loadEnv } from "vite";

export default defineConfig(({ mode }) => {
  const env = loadEnv(mode, process.cwd(), "SHAULA_");
  const target = env.SHAULA_API_TARGET || "http://127.0.0.1:8080";
  // These development-only headers stay in the Node process, never the bundle.
  const headers: Record<string, string> = env.SHAULA_DEV_BACKEND_TOKEN
    ? {
        "x-shaula-backend-auth": env.SHAULA_DEV_BACKEND_TOKEN,
        "x-shaula-actor": env.SHAULA_DEV_ACTOR || "web-developer",
        "x-shaula-scopes": env.SHAULA_DEV_SCOPES || "fleet.read,template.read,auth.read",
      }
    : {};
  return {
    plugins: [react(), tailwindcss()],
    resolve: { alias: { "@": fileURLToPath(new URL("./src", import.meta.url)) } },
    build: { sourcemap: false, minify: "oxc" },
    server: {
      proxy: Object.fromEntries(
        ["/api", "/livez", "/readyz"].map((path) => [path, { target, headers }]),
      ),
    },
  };
});
