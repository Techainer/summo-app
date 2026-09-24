import { readFileSync, readdirSync, statSync } from "node:fs";
import { join } from "node:path";

/**
 * Where a class is actually applied, as opposed to where one is being talked about.
 *
 * Three tests next door — `radius.test.ts`, `type.test.ts`, `spacing.test.ts` — ask the same
 * question of every component: which classes does this file apply, and is each one a token. They
 * need the same file walk and, far more importantly, the same idea of what counts as a call site.
 * Two copies of that idea agree until somebody fixes one; `legible.mjs` is shared between the two
 * screenshot suites for the same reason.
 */

/**
 * Components, and the plain modules that hold class strings for them.
 *
 * This walked `.tsx` only, which sounded right and was not. `components/settings/fields.ts` defines
 * the label, control, field and hint of every settings panel in the app as four exported strings,
 * and being a `.ts` file it was invisible to all three checks. It was carrying an off-grid `mt-3.5`
 * at the time, and a magic `ml-[162px]` that two panels had then copied by hand — exactly the drift
 * these tests exist to catch, in the one file best placed to spread it.
 *
 * Tests are left out: they quote offending classes on purpose, to prove the checks still bite.
 */
export function classFiles(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) return classFiles(path);
    if (/\.(test|spec)\.tsx?$/.test(path)) return [];
    return /\.tsx?$/.test(path) ? [path] : [];
  });
}

export function* callSites(dir: string): Generator<{ file: string; line: number; text: string }> {
  for (const file of classFiles(dir)) {
    for (const { line, text } of strings(readFileSync(file, "utf8"))) {
      yield { file, line, text };
    }
  }
}

/**
 * Every string literal in a source file, with the line it started on.
 *
 * Three narrowings, each of which arrived as a wrong answer from the version before it:
 *
 * **Only inside quotes.** Reading whole lines was fine while these checks walked `.tsx`, where
 * nearly every word shaped like a class is one. Reaching into `.ts` broke it at once: `storage.ts`
 * formats a file size into a local named `rounded`, and the radius check reported the variable. An
 * identifier is not a class, and the difference is the quotes around it.
 *
 * **Not inside comments.** Half the comments here quote the class strings they are about — `Field`
 * still documents the `rounded-lg border px-3 py-2 text-sm` it replaced, and that quotation is the
 * entire argument for `Field` existing. A migration script once rewrote those quotations and erased
 * the evidence for its own change. Skipping lines that begin with a star was enough for prose and
 * stopped being enough the moment quotes started mattering, because a backtick in a sentence and a
 * template literal are the same character.
 *
 * **Strings and comments read together, not in two passes.** Stripping comments first cuts
 * `href="https://…"` in half at the `//`. Stripping strings first swallows a comment that quotes
 * one. Only a single scan that knows which it is currently inside gets both right.
 *
 * A `${…}` hole is code again, so it is dropped. Whatever it interpolates is declared somewhere
 * these checks already walk.
 */
function* strings(source: string): Generator<{ line: number; text: string }> {
  let line = 1;
  let at = 0;

  while (at < source.length) {
    const here = source[at];

    if (here === "\n") {
      line++;
      at++;
      continue;
    }

    if (source.startsWith("//", at)) {
      while (at < source.length && source[at] !== "\n") at++;
      continue;
    }

    if (source.startsWith("/*", at)) {
      at += 2;
      while (at < source.length && !source.startsWith("*/", at)) {
        if (source[at] === "\n") line++;
        at++;
      }
      at += 2;
      continue;
    }

    if (here === '"' || here === "'" || here === "`") {
      const started = line;
      const quote = here;
      let text = "";
      at++;
      let depth = 0;
      while (at < source.length) {
        const char = source[at];
        if (char === "\\") {
          at += 2;
          continue;
        }
        if (source[at] === "\n") line++;
        // A hole in a template literal, which can itself contain quotes and braces.
        if (quote === "`" && source.startsWith("${", at)) {
          depth++;
          at += 2;
          continue;
        }
        if (depth > 0) {
          if (char === "{") depth++;
          if (char === "}") depth--;
          at++;
          continue;
        }
        if (char === quote) {
          at++;
          break;
        }
        text += char;
        at++;
      }
      if (text.trim()) yield { line: started, text };
      continue;
    }

    at++;
  }
}
