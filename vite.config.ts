import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Tauri expects a fixed dev port and clean output.
export default defineConfig({
    plugins: [react()],
    clearScreen: false,
    server: {
        port: 5173,
        strictPort: true,
        host: "127.0.0.1",
    },
    envPrefix: ["VITE_", "TAURI_"],
    build: {
        target: "es2021",
        minify: "esbuild",
        sourcemap: false,
    },
});