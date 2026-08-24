/**
 * Every screen, photographed.
 *
 * The other suites assert behaviour: a route renders, a drag lands, a file changes. None of them
 * can tell whether the result is *legible* — whether a heading collides with a chip at 390 px,
 * whether dark mode leaves grey text on a grey card, whether a screen with no data looks broken
 * rather than empty. That needs a human looking at a picture, and this produces the pictures.
 *
 * It also fails loudly on console errors and on obvious layout faults it *can* measure: horizontal
 * overflow, and text whose contrast against its own background is below the WCAG AA ratio. Those
 * two catch most of what a screenshot review would otherwise have to notice by eye.
 *
 * It boots a daemon of its own, like every other suite here, so it runs in CI rather than only on
 * the machine of whoever remembers. That is the point of the change: contrast and overflow were the
 * two things nothing enforced, checked by hand, on a good day.
 *
 *   node e2e/shots.mjs                                  # its own vault
 *   node e2e/shots.mjs http://127.0.0.1:7788 7788 <tok> # a daemon you are already debugging
 *
 * `SUMMO_LOCALE` picks the language the screens are photographed in; the four-language pass runs
 * inside this file either way.
 */
import { chromium } from "playwright";
import { mkdirSync } from "node:fs";

import { daemon } from "./daemon.mjs";

const engine = await daemon(process.argv, { name: "shots" });
const appUrl = engine.url;
const token = engine.token;
const locale = process.env.SUMMO_LOCALE ?? "vi-VN";

const OUT = "/tmp/shots";
mkdirSync(OUT, { recursive: true });

/** Routes worth a picture, with the hash the router uses. */
const SCREENS = [
  ["record", "/"],
  ["library", "/library"],
  ["meeting", null], // reached by clicking, since the id is generated
  ["notes", "/notes"],
  ["tasks", "/tasks"],
  ["agents", "/agents"],
  ["agenda", "/agenda"],
  ["chat", "/chat"],
  ["analytics", "/analytics"],
  ["people", "/people"],
  ["models", "/models"],
  ["settings", "/settings"],
];

const VIEWPORTS = [
  ["wide", { width: 1280, height: 860 }],
  ["narrow", { width: 390, height: 844 }],
];

const problems = [];
const browser = await chromium.launch();

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

for (const scheme of ["light", "dark"]) {
  for (const [width, viewport] of VIEWPORTS) {
    const context = await browser.newContext({ locale, viewport, colorScheme: scheme });
    // The quick tour is a first-run overlay. It is correct that it appears, and it covers a
    // quarter of the screen — so every picture of every screen would be a picture of the tour.
    // Marked as seen, the way it is for anybody who has used the app once.
    await context.addInitScript(() => window.localStorage.setItem("summo.tour", "done"));
    const page = await context.newPage();
    page.on("console", (m) => {
      if (m.type() === "error") problems.push(`console ${scheme}/${width}: ${m.text()}`);
    });
    page.on("pageerror", (e) => problems.push(`pageerror ${scheme}/${width}: ${e.message}`));

    for (const [name, route] of SCREENS) {
      if (route === null) continue;
      await page.goto(`${appUrl}/?token=${token}#${route}`, { waitUntil: "networkidle" });
      // The router paints after hydration; the shell header is the first thing that exists.
      await page.locator("header, main").first().waitFor({ timeout: 10000 });
      // Motion runs an entrance on most screens. Let it finish so the picture is the resting state.
      await page.waitForTimeout(700);
      await page.screenshot({ path: `${OUT}/${scheme}-${width}-${name}.png`, fullPage: false });

      const overflow = await page.evaluate(
        () => document.documentElement.scrollWidth - document.documentElement.clientWidth,
      );
      if (overflow > 2) {
        problems.push(`${scheme}/${width}/${name}: page scrolls sideways by ${overflow}px`);
      }

      // A key name where a sentence should be.
      //
      // `t()` falls back to the key it was given, so an untranslated string is not an error or a
      // blank — it is the literal text `settings.mt_where` sitting in a form. Two tests already
      // guard the catalogue's *contents*; this guards its *delivery*, which became a separate thing
      // the moment the catalogue was split into an eager half and a lazy one. A namespace filed in
      // the wrong half renders exactly this, on exactly these screens, and only on a cold load.
      const keyNames = await page.evaluate(() => {
        const shape = /^[a-z][a-z_]*\.[a-z_][a-z_0-9]*$/;
        const found = new Set();
        for (const element of document.querySelectorAll("body *")) {
          for (const node of element.childNodes) {
            if (node.nodeType !== Node.TEXT_NODE) continue;
            const text = (node.textContent ?? "").trim();
            if (shape.test(text)) found.add(text);
          }
        }
        return [...found];
      });
      for (const key of keyNames) {
        problems.push(`${scheme}/${width}/${name}: untranslated key on screen — ${key}`);
      }

      for (const run of await textColours(page)) {
        // WCAG AA: 4.5 for body text, 3.0 for large text (18.66px bold, or 24px).
        const large = run.size >= 24 || (run.size >= 18.66 && run.weight >= 700);
        const need = large ? 3 : 4.5;
        const got = contrast(run.fg, run.bg);
        if (got < need) {
          problems.push(
            `${scheme}/${width}/${name}: contrast ${got.toFixed(2)} < ${need} — "${run.text}" ${run.css}`,
          );
        }
      }
    }

    await context.close();
  }
}

await browser.close();
await engine.stop();

if (problems.length) {
  console.error(`\n${problems.length} problem(s):`);
  // Repeats are the same token used on every screen; showing each once is what is actionable.
  for (const p of [...new Set(problems)]) console.error(`  ${p}`);
  process.exit(1);
}
console.log(`shots in ${OUT}, no console errors, no overflow, contrast AA everywhere`);
