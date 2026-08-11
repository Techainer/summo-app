import js from "@eslint/js";
import reactHooks from "eslint-plugin-react-hooks";
import reactRefresh from "eslint-plugin-react-refresh";
import globals from "globals";
import tseslint from "typescript-eslint";

/**
 * What the linter is for here, and what it is not for.
 *
 * Formatting is Prettier's job and is not argued about. This file only carries rules that catch a
 * *bug* — the ones where the code is well-formed, reads fine, and is wrong.
 *
 * The rules below are on because each one has already cost this repository something:
 *
 * - `react-hooks/exhaustive-deps` — a stale closure in an effect is how a filtered list ends up
 *   showing the results of the filter before it.
 * - `no-floating-promises` — every daemon call returns a promise, and one that is neither awaited
 *   nor `void`-ed swallows its own failure. The app then looks like it did nothing.
 * - `no-misused-promises` — an `async` function passed where a `void` handler is expected, which is
 *   the same failure wearing a different hat.
 * - `switch-exhaustiveness-check` — the union types here (`Status`, `Lane`, `GroupBy`) grow, and a
 *   switch that silently falls through when they do is a screen that renders nothing.
 *
 * Type-aware linting costs a project build per run, which is why so many repositories skip it. The
 * four rules above are only possible with it, and they are the four worth having.
 */
export default tseslint.config(
  // `e2e` and the build configs are plain Node scripts outside the app's tsconfig, so the
  // type-aware rules have no program to consult for them.
  { ignores: ["dist", "src-tauri", "e2e", "*.config.ts", "*.config.js"] },
  js.configs.recommended,
  {
    files: ["**/*.{ts,tsx}"],
    // Scoped to TypeScript, not applied globally. A type-aware rule cannot run on a file the
    // compiler does not know about — this config itself, for one — and applying them everywhere
    // fails on that file rather than on anything wrong with it.
    extends: [...tseslint.configs.recommendedTypeChecked],
    languageOptions: {
      globals: globals.browser,
      parserOptions: {
        projectService: true,
        tsconfigRootDir: import.meta.dirname,
      },
    },
    plugins: {
      "react-hooks": reactHooks,
      "react-refresh": reactRefresh,
    },
    rules: {
      ...reactHooks.configs.recommended.rules,
      "react-refresh/only-export-components": ["warn", { allowConstantExport: true }],

      /*
       * A warning, not an error, and the count is capped rather than ignored — see `lint` in
       * package.json.
       *
       * Every one of the nineteen places this fires is the app reading from something outside
       * React on mount: fetching the library from the daemon, subscribing to a media query,
       * loading the i18n catalogue. The rule's own message names that as the legitimate use of an
       * effect; it cannot see through the `await` to tell that the `setState` is not synchronous.
       *
       * Left on because the shape it describes is worth noticing in review, and capped because a
       * warning nobody counts is a warning nobody reads.
       */
      "react-hooks/set-state-in-effect": "warn",

      "@typescript-eslint/no-floating-promises": "error",
      "@typescript-eslint/no-misused-promises": "error",
      // A `default` clause makes a switch exhaustive. Without this the rule demands every case be
      // written out *as well as* the catch-all, which is not what these switches are for — several
      // of them are "these three are special, everything else behaves the same way". What it still
      // catches, and what it is here for, is a switch with no default that quietly returns
      // `undefined` the day somebody adds a variant to the union.
      "@typescript-eslint/switch-exhaustiveness-check": [
        "error",
        { considerDefaultExhaustiveForUnions: true },
      ],

      // An unused argument named `_` is a deliberate placeholder; an unused one named anything else
      // is a leftover.
      "@typescript-eslint/no-unused-vars": [
        "error",
        { argsIgnorePattern: "^_", varsIgnorePattern: "^_" },
      ],

      // `tsc` already refuses an implicit `any`; this rule is about *explicit* ones, which are
      // sometimes the honest description of a value that arrives from outside the type system.
      "@typescript-eslint/no-explicit-any": "warn",
    },
  },
  {
    // An audio worklet runs on the audio thread, in a global scope that is neither a window nor a
    // worker: `AudioWorkletProcessor`, `registerProcessor` and `sampleRate` exist only there.
    files: ["public/*.js"],
    languageOptions: {
      globals: {
        AudioWorkletProcessor: "readonly",
        registerProcessor: "readonly",
        sampleRate: "readonly",
      },
    },
  },
  {
    // Tests reach into internals and assert on shapes the compiler cannot always follow.
    files: ["**/*.test.{ts,tsx}"],
    rules: {
      "@typescript-eslint/no-unsafe-assignment": "off",
      "@typescript-eslint/no-unsafe-member-access": "off",
      "@typescript-eslint/no-non-null-assertion": "off",
    },
  },
);
