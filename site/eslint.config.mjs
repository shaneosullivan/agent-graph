// The same rules as SyncAwesome's site (syncawesome/site/eslint.config.js).
import {defineConfig, globalIgnores} from "eslint/config";
import tsPlugin from "@typescript-eslint/eslint-plugin";
import tsParser from "@typescript-eslint/parser";
import reactHooks from "eslint-plugin-react-hooks";

const eslintConfig = defineConfig([
  globalIgnores([
    ".next/**",
    "out/**",
    "build/**",
    "node_modules/**",
    // Made by scripts, not by hand (as in .prettierignore).
    "next-env.d.ts",
    "public/viewer/**",
    "lib/viewer-shell.ts",
    "public/docs/reference/**",
  ]),
  {
    files: ["**/*.{ts,tsx}"],
    languageOptions: {
      parser: tsParser,
      parserOptions: {
        ecmaVersion: "latest",
        sourceType: "module",
        ecmaFeatures: {jsx: true},
      },
    },
    plugins: {
      "@typescript-eslint": tsPlugin,
      "react-hooks": reactHooks,
    },
    rules: {
      ...reactHooks.configs.recommended.rules,
      curly: ["error", "all"],
      "@typescript-eslint/array-type": ["error", {default: "generic"}],
      "@typescript-eslint/no-unused-vars": ["error", {argsIgnorePattern: "^_"}],
    },
  },
]);

export default eslintConfig;
