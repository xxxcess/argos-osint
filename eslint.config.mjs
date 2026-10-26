import tseslint from "typescript-eslint";
export default tseslint.config(
  ...tseslint.configs.recommended,
  { ignores: ["**/.next/**", "**/out/**", "**/next-env.d.ts"] },
  {
    rules: {
      "@typescript-eslint/no-unused-vars": [
        "error",
        { argsIgnorePattern: "^_" },
      ],
    },
  },
);
