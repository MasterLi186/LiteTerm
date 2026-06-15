import type { Config } from 'tailwindcss'

export default {
  content: ["./index.html", "./src/**/*.{js,ts,jsx,tsx}"],
  theme: {
    extend: {
      colors: {
        surface: {
          DEFAULT: '#0d1117',
          light: '#161b22',
          lighter: '#21262d',
          border: '#30363d',
        },
        accent: {
          cyan: '#00d4ff',
          green: '#00ff9f',
          purple: '#b388ff',
          red: '#ff5555',
          orange: '#ffb86c',
          yellow: '#f1fa8c',
        },
      },
    },
  },
  plugins: [],
} satisfies Config
