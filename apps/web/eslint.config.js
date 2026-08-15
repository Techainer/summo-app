import js from "@eslint/js";
import jsxA11y from "eslint-plugin-jsx-a11y";
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
  // `e2e`, `scripts` and the build configs are plain Node scripts outside the app's tsconfig, so
  // the type-aware rules have no program to consult for them.
  { ignores: ["dist", "src-tauri", "e2e", "scripts", "*.config.ts", "*.config.js"] },
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
      "jsx-a11y": jsxA11y,
    },
    rules: {
      ...reactHooks.configs.recommended.rules,
      ...jsxA11y.flatConfigs.recommended.rules,

      /*
       * Off, and it is the one rule in that set this project disagrees with.
       *
       * `no-autofocus` is right about a form on a page: focus that moves without being asked for
       * loses a screen reader user their place. It is wrong about the three places this codebase
       * uses it — a command palette, a rename field, and an inline edit — which are all dialogs
       * opened deliberately, where WAI-ARIA's own dialog pattern says focus *should* move to the
       * first field. A palette that opens without its input focused is broken, and working around
       * the rule with an effect and a ref is the same behaviour written less honestly.
       */
      "jsx-a11y/no-autofocus": "off",
      "react-refresh/only-export-components": ["warn", { allowConstantExport: true }],

      /*
       * A warning, and there are none left — `lint` runs with `--max-warnings 0`.
       *
       * It used to fire in nineteen places, all of them the app reading from something outside
       * React on mount, and the cap was raised each time somebody added a screen. That was the
       * wrong reading: about half were genuinely synchronous writes inside an effect — a media
       * query read twice, a sidebar state pushed back and forth across the breakpoint, three
       * pieces of dialog state cleared on close — and each one was a real extra render, one of
       * them a visible flash of the previous meeting's title.
       *
       * They are gone rather than suppressed. The async reads live behind `useLoad` and
       * `useRefresh`, which is one place instead of twenty; the resets are derived during render or
       * done by unmounting; the media query uses `useSyncExternalStore`, which is the API for it.
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
