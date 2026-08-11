import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

/**
 * No screen has words baked into it.
 *
 * Vietnamese was the first language this app spoke, so for a while the shortest way to write a
 * label was to type it. That kept working right up until the interface learned English, at which
 * point the task board drew "CHƯA LÀM / ĐANG LÀM / ĐANG CHỜ / XONG" under the heading "To do" —
 * and no amount of translating could fix it, because the words were not in a catalogue to
 * translate. A language somebody adds by dropping in a JSON file could never have reached them.
 *
 * It is caught by looking for Vietnamese letters, not for English ones, and that asymmetry is the
 * point: English text in the source is also a bug, but it is invisible to a test because every
 * identifier, class name and comment is English too. Vietnamese in a string literal has no
 * legitimate reason to exist outside the catalogues, which makes it the half of the problem a test
 * can actually hold. It is also the half that kept happening.
 */

const SRC = fileURLToPath(new URL("..", import.meta.url));

/**
 * What a line says to opt out, written at the line rather than in a list here.
 *
 * There is one true exception and it is worth allowing rather than working around: a language
 * offered in its own name. "Tiếng Việt" next to "English" and "日本語" is right in *every*
 * interface — translating a language picker into the language the user is trying to leave is the
 * bug, not the fix. Marking it at the site keeps the exception visible in review, where a list
 * kept over here would quietly grow.
 */
const EXEMPT = "i18n-exempt";

/** Letters that exist in Vietnamese and not in English. */
const VIETNAMESE = /[ăâđêôơưĂÂĐÊÔƠƯáàảãạấầẩẫậắằẳẵặéèẻẽẹếềểễệíìỉĩịóòỏõọốồổỗộớờởỡợúùủũụứừửữựýỳỷỹỵ]/i;

function sources(dir: string, out: string[] = []): string[] {
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) {
      // The catalogues are where the words belong. Tests carry Vietnamese fixtures on purpose —
      // a test for Vietnamese text that could not contain any would be testing nothing.
      if (entry.name !== "i18n") sources(path, out);
    } else if (/\.tsx?$/.test(entry.name) && !/\.test\.tsx?$/.test(entry.name)) {
      out.push(path);
    }
  }
  return out;
}

/**
 * Lines that would show a user Vietnamese words, as `line: text`.
 *
 * Line by line rather than by picking string literals out. A label in JSX is not in quotes at all —
 * `<Button>Huỷ</Button>` is text, and the first version of this test walked straight past a panel
 * full of them while matching half a file at a time on a stray backtick.
 *
 * Comments are blanked first, since the prose explaining a decision is often about Vietnamese, and
 * blanked line for line so the number reported is the number in the file.
 */
function baked(source: string): string[] {
  const withoutComments = source
    .replace(/\/\*[\s\S]*?\*\//g, (block) => block.replace(/[^\n]/g, " "))
    .replace(/^\s*\/\/.*$/gm, "");
  return withoutComments
    .split("\n")
    .map((line, i) => [i + 1, line.trim()] as const)
    .filter(([, line]) => VIETNAMESE.test(line) && !line.includes(EXEMPT))
    .map(([number, line]) => `${number}: ${line.slice(0, 70)}`);
}

describe("interface text", () => {
  const files = sources(SRC);

  it("has sources to check", () => {
    expect(files.length).toBeGreaterThan(20);
  });

  it("is never written into a component", () => {
    const found = files.flatMap((file) =>
      baked(readFileSync(file, "utf8")).map((where) => `${file.slice(SRC.length)}:${where}`),
    );
    expect(found, "these belong in src/i18n/*.json, with a key used here").toEqual([]);
  });
});
