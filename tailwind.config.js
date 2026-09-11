/** @type {import('tailwindcss').Config} */
export default {
    content: ["./index.html", "./src/**/*.{ts,tsx}"],
    theme: {
        extend: {
            fontFamily: {
                ui: [
                    "Inter",
                    "ui-sans-serif",
                    "system-ui",
                    "-apple-system",
                    "BlinkMacSystemFont",
                    "Segoe UI",
                    "PingFang TC",
                    "Noto Sans TC",
                    "sans-serif",
                ],
                mono: [
                    "JetBrains Mono",
                    "ui-monospace",
                    "SFMono-Regular",
                    "Menlo",
                    "Monaco",
                    "Consolas",
                    "monospace",
                ],
            },
            colors: {
                ink: {
                    50: "#f8fafc",
                    900: "#0b1220",
                    950: "#060a16",
                },
            },
        },
    },
    plugins: [],
};