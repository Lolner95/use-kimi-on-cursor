/** @type {import('tailwindcss').Config} */
export default {
  content: ["./index.html", "./src/**/*.{ts,html}"],
  theme: {
    extend: {
      colors: {
        feex: {
          // Primary brand — Feex coral red
          primary:        "#df3e2b",
          "primary-dark": "#c23520",
          "primary-light":"#e86657",
          // Secondary — warm peach
          secondary:      "#f8d1a7",
          "secondary-dark":"#e8b87a",
          // Surfaces — light warm
          "neutral-light":"#f5f5f5",   // body background
          canvas:         "#ffffff",   // card / surface
          "neutral-dark": "#ece6e6",   // border / divider
          // Text — warm on light
          "text-dark":    "#5a4a48",   // primary text
          "text-medium":  "#5a5a5a",   // secondary text
          "text-light":   "#8a8a8a",   // muted text
          // Semantic
          success:        "#4caf50",
          warning:        "#e8b87a",
        },
        kimi: {
          coral: "#df3e2b",
          peach: "#f8d1a7",
        },
        graphite: {
          600: "#8a8a8a",
          900: "#5a4a48",
        },
      },
      fontFamily: {
        sans:    ['"Baloo 2"', "Segoe UI Variable", "Segoe UI", "Arial", "sans-serif"],
        display: ['"Madimi One"', '"Baloo 2"', "Segoe UI", "sans-serif"],
        mono:    ["JetBrains Mono", "Consolas", "monospace"],
      },
      borderRadius: {
        feex:    "20px",
        "feex-sm":"12px",
        "feex-xs":"8px",
      },
      boxShadow: {
        feex:        "0 4px 20px rgba(0,0,0,0.06)",
        "feex-hover":"0 8px 30px rgba(0,0,0,0.10)",
        "feex-glow": "0 4px 16px rgba(223,62,43,0.30)",
        "green-glow":"0 0 12px rgba(76,175,80,0.35)",
        "red-glow":  "0 0 12px rgba(223,62,43,0.30)",
      },
      animation: {
        fade_in:      "fadeIn 0.35s ease-out",
        slide_up:     "slideUp 0.4s cubic-bezier(0.16,1,0.3,1)",
        shimmer:      "shimmer 4s linear infinite",
        "pulse-glow": "pulseGlow 2s ease-in-out infinite",
        "spin-slow":  "spin 3s linear infinite",
      },
      keyframes: {
        fadeIn: {
          "0%":   { opacity: "0", transform: "translateY(6px)" },
          "100%": { opacity: "1", transform: "translateY(0)" },
        },
        slideUp: {
          "0%":   { opacity: "0", transform: "translateY(16px) scale(0.98)" },
          "100%": { opacity: "1", transform: "translateY(0) scale(1)" },
        },
        shimmer: {
          "0%":   { backgroundPosition: "200% center" },
          "100%": { backgroundPosition: "-200% center" },
        },
        pulseGlow: {
          "0%, 100%": { boxShadow: "0 0 0 0 rgba(76,175,80,0.45)" },
          "50%":       { boxShadow: "0 0 0 7px rgba(76,175,80,0)" },
        },
      },
    },
  },
  plugins: [],
};
