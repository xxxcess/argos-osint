import reactHooks from "eslint-plugin-react-hooks";
import tseslint from "typescript-eslint";
export default tseslint.config(
  ...tseslint.configs.recommended,
  reactHooks.configs.flat.recommended,
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
