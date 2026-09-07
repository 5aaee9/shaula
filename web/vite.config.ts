import { fileURLToPath, URL } from "node:url";
import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import { defineConfig, loadEnv } from "vite";
import { oidcDevelopmentGuard } from "./dev-auth.ts";

export default defineConfig(({ mode }) => {
  const env = loadEnv(mode, process.cwd(), "SHAULA_");
  const target = env.SHAULA_API_TARGET || "http://127.0.0.1:8080";
  const proxy = Object.fromEntries(
    ["/api", "/auth/oidc/", "/livez", "/readyz"].map((path) => [path, { target }]),
  );
  return {
    plugins: [oidcDevelopmentGuard(target), react(), tailwindcss()],
    resolve: { alias: { "@": fileURLToPath(new URL("./src", import.meta.url)) } },
    build: { sourcemap: false, minify: "oxc" },
    server: {
      headers: { "Cache-Control": "private, no-store" },
      ws: false,
      hmr: false,
      proxy,
    },
    preview: { proxy, headers: { "Cache-Control": "private, no-store" } },
  };
});
