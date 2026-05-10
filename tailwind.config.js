/** @type {import('tailwindcss').Config} */
module.exports = {
  content: [
    "./crates/rusterando-frontend/src/**/*.rs",
  ],
  theme: {
    extend: {
      colors: {
        primary: "#C8102E",
        "primary-dark": "#8B0A1F",
        secondary: "#2E7D32",
        accent: "#F5C518",
      },
    },
  },
  plugins: [],
};
