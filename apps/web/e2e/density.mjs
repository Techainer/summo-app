/**
 * No screen may be a heading and then several hundred pixels of nothing.
 *
 * This is the defect that produced the redesign: `shots.mjs` passed — no overflow, AA contrast
 * everywhere — on screens a person looked at and called empty. Contrast checks read the pixels that
 * *are* there and have nothing to say about the ones that are not, so the emptiness was invisible
 * to every automated check in the repository while being the first thing a human noticed.
 *
 * The rule here: on each screen, measure from the bottom of the last visible content to the bottom
 * of the scrolling pane. If more than a third of the pane is dead space below everything, and the
 * pane does not scroll (so there is nothing further down to justify it), the screen fails.
 *
 * A third rather than a half, and measured against the *pane* rather than the window, so a genuinely
 * short screen — an empty state, a settings form — passes as long as it fills or centres itself.
 * `Empty full` exists for exactly that and is why the empty screens are not failures here.
 */
import { chromium } from "playwright";

import { daemon } from "./daemon.mjs";

const engine = await daemon(process.argv, { name: "density" });
const { url: appUrl, port, token } = engine;

/** Every screen in the work group plus the setup group, by the label in the sidebar. */
const SCREENS = [
  "Trang chính",
  "Ghi",
  "Kho",
  "Việc",
  "Lịch",
  "Thống kê",
  "Ghi chú",
  "Hỏi đáp",
  "Giọng nói",
  "Agent",
  "Mô hình",
  "Cài đặt",
];

/** How much of the pane may be empty below the last thing on it. */
const SLACK = 1 / 3;

const browser = await chromium.launch();
const context = await browser.newContext({
  locale: "vi-VN",
  viewport: { width: 1280, height: 860 },
});
const page = await context.newPage();
const problems = [];
page.on("pageerror", (e) => problems.push(`pageerror: ${e.message}`));

await page.goto(`${appUrl}?port=${port}&token=${token}`, { waitUntil: "networkidle" });
await page.locator("header").waitFor({ timeout: 10000 });

for (const label of SCREENS) {
  await page
    .getByRole("navigation", { name: "Màn hình" })
    .getByRole("button", { name: label, exact: true })
    .click();
  // Motion: cards stagger in, and measuring mid-animation measures the wrong height.
  await page.waitForTimeout(700);

  const verdict = await page.evaluate(
    ([slack]) => {
      const main = document.querySelector("main");
      if (!main) return { ok: false, why: "no <main>" };

      // The pane is `main`, always. An earlier version of this file picked "the first scrollable
      // element inside main" and then asked whether that element scrolled — which it did, by
      // construction. Every screen passed and the suite measured nothing.
      const pane = main;
      const box = pane.getBoundingClientRect();
      if (box.height < 200) return { ok: true, why: "pane too short to judge" };

      // If the screen itself scrolls there is content below the fold, and empty space at the bottom
      // of the viewport is not empty space on the screen.
      if (pane.scrollHeight > pane.clientHeight + 4)
        return { ok: true, why: `scrolls ${pane.scrollHeight}/${pane.clientHeight}` };

      const visible = (el) => {
        const style = getComputedStyle(el);
        return style.visibility !== "hidden" && style.opacity !== "0";
      };

      /**
       * A leaf that puts something on screen: text, an image, a control.
       *
       * `data-ink` is how a component says "this draws something" when nothing else here can tell.
       * The list of tags below cannot see a drawing built out of `<span>`s and gradients, and the
       * `SVG` in it has never matched anything either — a Lucide icon is an `<svg>` with `<path>`
       * children, so it is not a leaf, and an SVG element's `tagName` is lowercase in an HTML
       * document besides. Rather than teach this walker to recognise art, the art declares itself:
       * see `components/ui/Spot.tsx`.
       *
       * It matters because an illustrated empty state is *mostly* drawing. Measured from the top of
       * its text instead, the composition looks lopsided and the screen is reported as a hole.
       */
      const isInk = (el) =>
        el.dataset.ink !== undefined ||
        (el.childElementCount === 0 &&
          ((el.textContent ?? "").trim().length > 0 ||
            ["IMG", "SVG", "CANVAS", "INPUT", "TEXTAREA"].includes(el.tagName)));

      /** The vertical extent of everything drawn inside `root`. */
      const inkBounds = (root) => {
        let top = Infinity;
        let bottom = -Infinity;
        const walk = document.createTreeWalker(root, NodeFilter.SHOW_ELEMENT);
        for (let el = walk.nextNode(); el; el = walk.nextNode()) {
          const rect = el.getBoundingClientRect();
          if (rect.height < 1 || rect.width < 1 || !isInk(el) || !visible(el)) continue;
          top = Math.min(top, rect.top);
          bottom = Math.max(bottom, rect.bottom);
        }
        return { top, bottom };
      };

      let lowest = box.top;
      const walk = document.createTreeWalker(pane, NodeFilter.SHOW_ELEMENT);
      for (let el = walk.nextNode(); el; el = walk.nextNode()) {
        const rect = el.getBoundingClientRect();
        if (rect.width < 1 || rect.height < 1 || !visible(el)) continue;
        const style = getComputedStyle(el);

        // Four things count as reaching the bottom of the space they occupy.

        // 1. A box that scrolls its own contents. It was sized to fill that space and there is more
        //    inside it than is visible — the transcript and the note list are this.
        const scroller =
          el.scrollHeight > el.clientHeight + 4 &&
          ["auto", "scroll"].includes(style.overflowY) &&
          rect.height > 120;

        // 2. A box that paints itself — a border or a fill — and takes at least half the pane. A
        //    kanban lane is this: the lane *is* the affordance, and a lane drawn down the screen is
        //    not empty space even when it holds two cards. Half, so the version of the tasks board
        //    that produced this suite (180px lanes over 330px of background) would still fail.
        const painted =
          (parseFloat(style.borderTopWidth) > 0 ||
            !["rgba(0, 0, 0, 0)", "transparent"].includes(style.backgroundColor)) &&
          rect.height >= box.height * 0.5;

        // 3. A container that fills the pane and centres its contents. An empty state placed in the
        //    middle of a column is a composition, not a hole, and `Empty full` exists to make one.
        let centred = false;
        if (rect.height >= box.height * 0.85) {
          const ink = inkBounds(el);
          if (ink.bottom > -Infinity) {
            const above = ink.top - rect.top;
            const below = rect.bottom - ink.bottom;
            centred = Math.abs(above - below) <= rect.height * 0.15;
          }
        }

        // 4. Anything that draws text or an image, wherever it is.
        if (!scroller && !painted && !centred && !isInk(el)) continue;
        lowest = Math.max(lowest, Math.min(rect.bottom, box.bottom));
      }

      const below = box.bottom - lowest;
      return {
        ok: below <= box.height * slack,
        empty: Math.round(below),
        height: Math.round(box.height),
      };
    },
    [SLACK],
  );

  if (!verdict.ok) {
    problems.push(
      `${label}: ${verdict.empty}px of a ${verdict.height}px pane is empty below everything on it`,
    );
  } else {
    console.log(
      `${label}: ok (${
        verdict.empty !== undefined
          ? `${verdict.empty}px of ${verdict.height}px empty`
          : verdict.why
      })`,
    );
  }
}

// The bottom bar is the mobile half of the same problem: below the breakpoint the four
// destinations must be reachable without opening a sheet, and must not sit on top of the content.
{
  const narrow = await browser.newContext({
    locale: "vi-VN",
    viewport: { width: 390, height: 844 },
  });
  const small = await narrow.newPage();
  await small.goto(`${appUrl}?port=${port}&token=${token}`, { waitUntil: "networkidle" });
  const bar = small.getByTestId("bottom-bar");
  await bar.waitFor({ timeout: 10000 });

  const tabs = await bar.getByRole("button").allInnerTexts();
  console.log(`bottom bar: ${tabs.filter(Boolean).join(" | ")}`);
  if (tabs.filter(Boolean).length !== 4) {
    problems.push(`bottom bar has ${tabs.filter(Boolean).length} labelled tabs, expected 4`);
  }

  // Overlap: the bar is in the column flow, not fixed over it, so the last row of content must end
  // above the bar's top edge.
  const overlaps = await small.evaluate(() => {
    const bar = document.querySelector('[data-testid="bottom-bar"]');
    const main = document.querySelector("main");
    if (!bar || !main) return "missing bar or main";
    const barTop = bar.getBoundingClientRect().top;
    return main.getBoundingClientRect().bottom > barTop + 1 ? "main runs under the bar" : null;
  });
  if (overlaps) problems.push(`bottom bar: ${overlaps}`);

  await narrow.close();
}

await context.close();
await browser.close();
await engine.stop();

if (problems.length > 0) {
  console.error(problems.join("\n"));
  process.exit(1);
}
console.log("density ok");
