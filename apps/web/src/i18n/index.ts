/**
 * Translation.
 *
 * Summo was written in Vietnamese first, which was the right call — it is built for the Vietnamese
 * market and a product designed in English and translated afterwards reads like one. But hardcoded
 * strings are cheap to add and expensive to remove, so this goes in before there are thousands of
 * them rather than after.
 *
 * Four decisions worth stating.
 *
 * **A missing key renders the key, never blank.** A blank label is invisible: it ships, nobody
 * notices, and a user finds an unlabelled button. `meeting.stop` on screen is ugly and obviously
 * wrong, which is the point.
 *
 * **Vietnamese is the source language, not just another locale.** It is the fallback of last
 * resort, so an English bundle missing a key shows Vietnamese rather than nothing. Half-translated
 * is a normal state for a locale a user contributed and it must degrade gracefully.
 *
 * **Anyone can add a language by dropping a JSON file** into `~/.summo/locales/`. No build step, no
 * pull request, no rebuild. That is how a tool used across a dozen countries actually gets
 * translated — see {@link mergeCatalogs}.
 *
 * **Plurals are a suffix convention, not a library.** Vietnamese has no plural agreement at all;
 * English has two forms. `key_one`/`key_other` covers both and costs nothing. A language with six
 * forms needs `Intl.PluralRules`, which is why {@link plural} routes through it rather than
 * comparing to 1.
 */

/** A flat key-to-string map. Nested JSON is flattened on load, so `t("meeting.stop")` works. */
export type Catalog = Record<string, string>;

/**
 * The language the strings were written in, and the fallback of last resort.
 *
 * Not the same thing as the default. Vietnamese is where the wording came from, so a key a locale
 * has not translated falls back to it rather than to a key name — but a first-time user with no
 * saved choice and an English browser gets {@link DEFAULT}.
 */
export const SOURCE = "vi";

/**
 * What to show when nothing else decides.
 *
 * English, because it is the language most people who arrive without a preference can read. The
 * browser's own language still wins over it, so a Vietnamese machine still opens in Vietnamese —
 * this only settles the case where the browser asks for something Summo does not have.
 */
export const DEFAULT = "en";

export interface Language {
  code: string;
  /** The language's name in that language — "Tiếng Việt", not "Vietnamese". */
  label: string;
}

/**
 * Languages that ship with the app, fetched one at a time.
 *
 * A dynamic import each, rather than four static ones, and the difference is what a first-time
 * visitor downloads: the four catalogues used to sit in the entry chunk together, so every user
 * carried three languages they cannot read before the app painted anything. Thirty kilobytes of it,
 * and growing by four every time somebody wrote one sentence of new copy.
 *
 * The cost of doing it this way is that `t()` has something to wait for, which is why
 * {@link ready} and {@link ensure} exist and why the provider does not render until the answer is
 * yes. That wait is one fetch of a few kB from the same origin — the daemon on localhost, or the
 * shell's own bundle — and it is overlapped with the engine handshake, which takes far longer.
 *
 * User-supplied catalogs from `~/.summo/locales/` are added on top at runtime and are not here:
 * they arrive over the network from the daemon, already parsed.
 */
const FILES: Record<string, () => Promise<{ default: unknown }>> = {
  vi: () => import("./vi.json"),
  en: () => import("./en.json"),
  ja: () => import("./ja.json"),
  zh: () => import("./zh.json"),
};

/** The tags {@link FILES} can serve, for code that only needs to know what exists. */
export const BUILT_IN_CODES: string[] = Object.keys(FILES);

/** Flattened catalogs that have arrived. Keyed by tag, filled by {@link ensure}. */
const loaded = new Map<string, Catalog>();

/** Loads in flight, so two components asking at once cause one fetch rather than two. */
const inflight = new Map<string, Promise<void>>();

function fetchCatalog(code: string): Promise<void> {
  if (loaded.has(code)) return Promise.resolve();
  const running = inflight.get(code);
  if (running) return running;

  const file = FILES[code];
  if (!file) return Promise.resolve();

  const task = file()
    .then(
      (module) => {
        loaded.set(code, flatten(module.default));
      },
      () => {
        // A chunk that will not load must not leave the app blank forever. Nothing is cached, so a
        // later attempt — switching language and switching back — tries again, and in the meantime
        // the screen renders key names, which is the same thing a missing key has always done.
      },
    )
    .finally(() => inflight.delete(code));

  inflight.set(code, task);
  return task;
}

/**
 * Which shipped catalogs {@link catalogFor} will layer for this locale.
 *
 * A built-in locale needs only itself. That is not an optimisation taken on faith: `i18n.test.ts`
 * holds every shipped catalogue to the source's keys in *both* directions, so `ja.json` cannot be
 * missing a key that `vi.json` has. Loading Vietnamese underneath Japanese would be downloading a
 * fallback that can never be reached.
 *
 * A locale that is *not* built in is a file somebody dropped into `~/.summo/locales/`, and those
 * are partial by nature — ten strings translated and the rest not. That one does get the source
 * underneath it, which is the case the fallback was written for.
 */
export function layersOf(locale: string): string[] {
  const primary = locale.split("-")[0];
  if (locale in FILES) return [locale];
  if (primary && primary in FILES) return [primary];
  return [SOURCE];
}

/** Whether {@link catalogFor} can answer for this locale right now, without waiting. */
export function ready(locale: string): boolean {
  return layersOf(locale).every((code) => loaded.has(code));
}

/** Fetch whatever this locale needs. Resolves even when a chunk fails — see {@link fetchCatalog}. */
export function ensure(locale: string): Promise<void> {
  return Promise.all(layersOf(locale).map(fetchCatalog)).then(() => undefined);
}

/// `zh` is Simplified Chinese, and is deliberately the bare tag rather than `zh-Hans`: a browser
/// asking for `zh-CN` or `zh-SG` finds it by primary subtag, and a Traditional catalogue can be
/// added later as `zh-Hant` without renaming this one out from under anyone's saved choice.
export const BUILT_IN_LANGUAGES: Language[] = [
  { code: "vi", label: "Tiếng Việt" },
  { code: "en", label: "English" },
  { code: "ja", label: "日本語" },
  { code: "zh", label: "简体中文" },
];

/**
 * Turn nested JSON into dotted keys.
 *
 * The files are nested because that is what a translator can read; the lookup is flat because a
 * dotted key is a single map access rather than a walk that has to handle a missing branch at every
 * level.
 */
export function flatten(source: unknown, prefix = ""): Catalog {
  const out: Catalog = {};
  if (typeof source !== "object" || source === null) return out;

  for (const [key, value] of Object.entries(source as Record<string, unknown>)) {
    const path = prefix ? `${prefix}.${key}` : key;
    if (typeof value === "string") {
      out[path] = value;
    } else if (typeof value === "object" && value !== null) {
      Object.assign(out, flatten(value, path));
    }
    // Numbers and booleans are dropped: a catalog value that is not a string is a mistake in the
    // file, and coercing it would hide it.
  }
  return out;
}

/**
 * Layer catalogs, later winning over earlier.
 *
 * The order is always source → built-in locale → user file, so a user who translates ten strings
 * gets those ten and the rest of the app in Vietnamese, rather than an app that is ninety percent
 * key names.
 */
export function mergeCatalogs(...catalogs: Catalog[]): Catalog {
  return Object.assign({}, ...catalogs) as Catalog;
}

/** Values substituted into `{placeholders}`. */
export type Values = Record<string, string | number>;

/**
 * Substitute `{name}` placeholders.
 *
 * A placeholder with no value is left as-is rather than replaced with `undefined`. Seeing `{count}`
 * on screen tells whoever sees it exactly which key and which variable to fix; "undefined" does
 * not.
 */
export function interpolate(template: string, values?: Values): string {
  if (!values) return template;
  return template.replace(/\{(\w+)\}/g, (whole, name: string) => {
    const value = values[name];
    return value === undefined ? whole : String(value);
  });
}

/**
 * Pick the plural form for `count` in `locale`.
 *
 * Routed through `Intl.PluralRules` rather than `count === 1`, because Arabic has six forms and
 * Polish has four, and a hand-rolled comparison silently produces wrong grammar in those languages
 * rather than an error anyone would notice.
 */
export function plural(
  catalog: Catalog,
  key: string,
  count: number,
  locale: string,
): string | undefined {
  let category: string;
  try {
    category = new Intl.PluralRules(locale).select(count);
  } catch {
    // An unknown locale tag must not take the screen down.
    category = count === 1 ? "one" : "other";
  }
  return catalog[`${key}_${category}`] ?? catalog[`${key}_other`] ?? catalog[key];
}

/** A bound translator. */
export interface Translator {
  locale: string;
  t: (key: string, values?: Values) => string;
  /** Translate with a count, choosing the right plural form. */
  n: (key: string, count: number, values?: Values) => string;
  /** Whether a key exists, for code that wants to fall back to something else entirely. */
  has: (key: string) => boolean;
}

/**
 * Build a translator over a catalog.
 *
 * `onMissing` fires once per key, not once per render — a missing label in a list of two hundred
 * meetings would otherwise produce two hundred identical warnings a second.
 */
export function translator(
  locale: string,
  catalog: Catalog,
  onMissing?: (key: string) => void,
): Translator {
  const reported = new Set<string>();

  const lookup = (key: string): string | undefined => {
    const found = catalog[key];
    if (found === undefined && onMissing && !reported.has(key)) {
      reported.add(key);
      onMissing(key);
    }
    return found;
  };

  return {
    locale,
    t: (key, values) => interpolate(lookup(key) ?? key, values),
    n: (key, count, values) => {
      const template = plural(catalog, key, count, locale);
      if (template === undefined && onMissing && !reported.has(key)) {
        reported.add(key);
        onMissing(key);
      }
      return interpolate(template ?? key, { count, ...values });
    },
    has: (key) => catalog[key] !== undefined,
  };
}

/**
 * The locale to start in.
 *
 * A saved choice wins; otherwise the browser's language, matched on the primary subtag so `en-GB`
 * finds `en`; otherwise {@link DEFAULT}. Never throws — a locale lookup failing must not stop the
 * app from rendering.
 */
export function detectLocale(available: string[], saved?: string | null): string {
  if (saved && available.includes(saved)) return saved;

  // Typed explicitly: `Array.isArray` on a `readonly string[]` narrows to `any[]`, which then
  // spreads `any` through everything downstream of this loop.
  const candidates: readonly string[] =
    typeof navigator !== "undefined" && Array.isArray(navigator.languages)
      ? navigator.languages
      : [];

  for (const tag of candidates) {
    if (available.includes(tag)) return tag;
    const primary = tag.split("-")[0];
    if (primary && available.includes(primary)) return primary;
  }
  if (available.includes(DEFAULT)) return DEFAULT;
  return available.includes(SOURCE) ? SOURCE : (available[0] ?? DEFAULT);
}

/**
 * Build the catalog for a locale, from what has been loaded.
 *
 * Synchronous, and deliberately so: `t()` is called during render and cannot await. Everything it
 * needs is decided by {@link layersOf} and fetched by {@link ensure} before the provider renders,
 * so this is a map lookup rather than a promise. A locale nothing has loaded yields an empty
 * catalog, and the translator renders key names — visibly wrong rather than blank.
 *
 * `extra` is the user-supplied file, which wins over everything: someone who dislikes our wording
 * for their own language should be able to change it without asking.
 */
export function catalogFor(locale: string, extra?: Catalog): Catalog {
  const layers: Catalog[] = layersOf(locale).map((code) => loaded.get(code) ?? {});
  if (extra) layers.push(extra);
  return mergeCatalogs(...layers);
}
