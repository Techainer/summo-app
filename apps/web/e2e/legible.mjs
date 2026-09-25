/**
 * Is what is on the screen readable, and does it fit.
 *
 * Extracted from `shots.mjs` when a second suite needed the same two questions. Copying them would
 * have meant two definitions of "AA contrast" that agree until somebody fixes one — and the whole
 * argument of that suite is that these are the checks a person cannot be relied on to do by eye.
 */

/**
 * Relative luminance per WCAG, from `[r, g, b]` in 0–255.
 */
function luminance([r, g, b]) {
  const channel = (v) => {
    const s = v / 255;
    return s <= 0.03928 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b);
}

function contrast(fg, bg) {
  const a = luminance(fg);
  const b = luminance(bg);
  return (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05);
}

/**
 * Colours of every visible text run, paired with the background actually painted behind it.
 *
 * Three things make this harder than reading `backgroundColor`:
 *
 * - Nearly every element is transparent, so the background is an ancestor's.
 * - Tokens are `oklab(… / 0.4)` and `rgba(…, 0.08)` — translucent tints over another colour. Taking
 *   the first non-`transparent` value scores text against a layer you can see straight through,
 *   which reported half the interface as unreadable when it is not. Each translucent layer has to
 *   be composited onto what is behind it.
 * - A gradient is `background-image`, not `background-color`, so an element painted with one looks
 *   transparent. The record button is exactly that, and walking past it compared white text against
 *   the page rather than against the button.
 *
 * Colours are resolved through a canvas because it is the only way to turn `oklab()` and
 * `color-mix()` into sRGB without reimplementing the colour module.
 */
async function textColours(page) {
  return page.evaluate(() => {
    const canvas = document.createElement("canvas");
    canvas.width = canvas.height = 1;
    const ctx = canvas.getContext("2d", { willReadFrequently: true });

    /** Any CSS colour → `[r, g, b, a]`, alpha 0–1. */
    const rgba = (colour) => {
      ctx.clearRect(0, 0, 1, 1);
      ctx.fillStyle = "#000";
      ctx.fillStyle = colour; // Invalid values leave the previous one; black is a safe fallback.
      ctx.fillRect(0, 0, 1, 1);
      // `getImageData` un-premultiplies for us, so these are already straight RGB with a separate
      // alpha — dividing by alpha again turned a 10% amber tint into a colour brighter than white.
      const [r, g, b, a] = ctx.getImageData(0, 0, 1, 1).data;
      return [r, g, b, a / 255];
    };

    /** `over` painted on top of `under`, both opaque-corrected. */
    const composite = (over, under) => {
      const a = over[3];
      return [
        over[0] * a + under[0] * (1 - a),
        over[1] * a + under[1] * (1 - a),
        over[2] * a + under[2] * (1 - a),
        1,
      ];
    };

    const out = [];
    for (const el of document.querySelectorAll("body *")) {
      const text = [...el.childNodes]
        .filter((n) => n.nodeType === 3)
        .map((n) => n.textContent.trim())
        .join("")
        .trim();
      if (!text) continue;
      const box = el.getBoundingClientRect();
      if (box.width < 2 || box.height < 2) continue;
      const style = getComputedStyle(el);
      if (style.visibility === "hidden" || Number(style.opacity) === 0) continue;

      // What is painted under this text, top layer first.
      //
      // `elementsFromPoint` rather than a walk up `parentElement`, because the layer behind a
      // selected segmented-control option is an absolutely-positioned *sibling* that Motion moves
      // between options — no ancestor of the label has that colour, and comparing against the page
      // instead called every selected white label unreadable.
      const cx = Math.min(Math.max(box.left + box.width / 2, 1), innerWidth - 1);
      const cy = Math.min(Math.max(box.top + box.height / 2, 1), innerHeight - 1);
      const stack = document.elementsFromPoint(cx, cy);
      const start = stack.indexOf(el);
      if (start === -1) continue; // Covered by something else; not what the user reads.

      const layers = [];
      let gradient = false;
      // From the element itself: text paints on top of its own background, so a green button with
      // white text must be scored against the green and not against the page behind the button.
      for (const node of stack.slice(start)) {
        const s = getComputedStyle(node);
        if (s.backgroundImage && s.backgroundImage !== "none") {
          // A gradient's colour cannot be sampled this way; stop trusting the stack.
          gradient = true;
          break;
        }
        const layer = rgba(s.backgroundColor);
        if (layer[3] === 0) continue;
        layers.push(layer);
        if (layer[3] === 1) break;
      }
      if (gradient) continue;

      let bg = [255, 255, 255, 1];
      for (const layer of layers.reverse()) bg = composite(layer, bg);

      const fg = composite(rgba(style.color), bg);
      out.push({
        text: text.slice(0, 40),
        fg,
        bg,
        css: `${style.color} on ${getComputedStyle(el).backgroundColor}`,
        size: parseFloat(style.fontSize),
        weight: Number(style.fontWeight) || 400,
      });
    }
    return out;
  });
}

/**
 * Both checks against whatever is currently rendered, appended to `problems`.
 *
 * `label` is how a failure names itself — the caller knows whether that is a screen at a viewport
 * or a primitive in a state, and neither should have to be spelled here.
 */
/**
 * Things a developer writes to a checker, which a user should never be shown.
 *
 * `i18n-exempt` marks a string that deliberately is not translated, and it is written as a trailing
 * `//` comment. Children of a JSX element are text, not code, so the same marker put *inside* one
 * renders — and it did: three labels on the gallery page read "đã chọn // i18n-exempt" and
 * "hành động // i18n-exempt" through a release. It satisfied `no-baked-in-text.test.ts`, which
 * looks for the marker on the line and asks nothing about where on the line it is, and it was under
 * the fold of a screenshot that stopped at one screenful. Two checks agreeing that nothing was
 * wrong.
 *
 * Checked against what is painted rather than against the source, because "it rendered" is the
 * actual failure and is the one thing neither of those checks was looking at.
 */
const NEVER_RENDERED = [/i18n-exempt/, /\bTODO\b/, /\beslint-disable/, /\bundefined\b/];

export async function legible(page, label, problems) {
  const overflow = await page.evaluate(
    () => document.documentElement.scrollWidth - document.documentElement.clientWidth,
  );
  if (overflow > 2) problems.push(`${label}: page scrolls sideways by ${overflow}px`);

  const shown = await page.evaluate(() => document.body.innerText);
  for (const marker of NEVER_RENDERED) {
    const hit = marker.exec(shown);
    if (!hit) continue;
    const around = shown.slice(Math.max(0, hit.index - 30), hit.index + 40).replace(/\s+/g, " ");
    problems.push(`${label}: "${hit[0]}" is on the screen — "…${around}…"`);
  }

  for (const control of await crushed(page)) {
    problems.push(
      `${label}: a ${control.what} is ${control.width}px wide — "${control.value}" is all that fits`,
    );
  }

  for (const run of await textColours(page)) {
    // WCAG AA: 4.5 for body text, 3.0 for large text (18.66px bold, or 24px).
    const large = run.size >= 24 || (run.size >= 18.66 && run.weight >= 700);
    const need = large ? 3 : 4.5;
    const got = contrast(run.fg, run.bg);
    if (got < need) {
      problems.push(`${label}: contrast ${got.toFixed(2)} < ${need} — "${run.text}" ${run.css}`);
    }
  }
}

/**
 * The floor under which a field somebody types into stops being usable.
 *
 * Eighty pixels shows about six characters. A path, a name or a passphrase in six characters is a
 * field you cannot read back or correct, and the failure is not that something broke — it is that a
 * row ran out of room and the control was the part that gave.
 *
 * This exists because the screenshot audit passed exactly that. The sync panel's folder row put a
 * 150px label, a path input and a "choose folder" button on one line; on a 390px screen the button
 * took what it needed and the input was left showing `/mnt/`. Nothing overflowed, every contrast
 * was fine, and both suites were green. A phone screenshot showed it in a second, which is the
 * point — a person looked and a machine had not been asked to.
 *
 * Measured at 96px, the narrowest deliberate field in the app (a two-digit thread count), so the
 * floor sits below every real one with room to spare.
 */
const MIN_FIELD = 80;

async function crushed(page) {
  return page.evaluate((floor) => {
    // Checkboxes and radios are `sr-only` inputs painted by a sibling, so their own box is one
    // pixel by design and means nothing. Ranges and colour wells are dragged, not typed into.
    const TYPED = ["text", "password", "search", "email", "url", "tel", "number", ""];
    const found = [];
    for (const el of document.querySelectorAll("input, select, textarea")) {
      if (el.offsetParent === null) continue;
      const tag = el.tagName.toLowerCase();
      if (tag === "input" && !TYPED.includes(el.getAttribute("type") ?? "")) continue;
      const width = Math.round(el.getBoundingClientRect().width);
      if (width >= floor) continue;
      found.push({
        what: `${tag}[${el.getAttribute("type") ?? ""}]`,
        width,
        // What a reader would actually see in it, so the message names the field rather than
        // describing an anonymous box.
        value: (el.value || el.getAttribute("placeholder") || el.getAttribute("aria-label") || "")
          .toString()
          .slice(0, 40),
      });
    }
    return found;
  }, MIN_FIELD);
}
