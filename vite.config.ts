import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],
  base: "./",
  clearScreen: false,
  build: {
    outDir: "dist",
    emptyOutDir: true
  },
  server: {
    strictPort: true,
    port: 1420
  }
});
