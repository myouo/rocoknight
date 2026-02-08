import type { Config } from "tailwindcss";

export default {
  content: ["./index.html", "./src/**/*.{ts,tsx}"] ,
  theme: {
    extend: {
      colors: {
        ink: "#0B0D12",
        steel: "#1E2430",
        ember: "#E07A5F",
        mist: "#EDE6DA",
        slate: "#94A3B8"
      },
      fontFamily: {
        display: ["Space Grotesk", "ui-sans-serif", "system-ui"],
        body: ["Source Sans 3", "ui-sans-serif", "system-ui"]
      }
    }
  },
  plugins: []
} satisfies Config;
