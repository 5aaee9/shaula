// Isolated presentation tests only. The normal dev/preview config always authenticates.
import { fileURLToPath, URL } from "node:url";
import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";
export default defineConfig({
  // The dev-auth test starts the normal Vite config alongside this server.
  cacheDir: "node_modules/.vite-components",
  plugins: [react(), tailwindcss()],
  resolve: { alias: { "@": fileURLToPath(new URL("../src", import.meta.url)) } },
});
