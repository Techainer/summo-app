/**
 * Subscribing to a calendar, in the browser, against a real calendar server.
 *
 * The unit tests cover which URLs are refused and the daemon test covers the fetch. What neither
 * can cover is the part that was actually missing for a year: the only way to add a calendar was to
 * type the *path of a file*, which a browser cannot produce and which stops being true the day
 * after it is exported. This drives the form a person uses.
 *
 * The calendar is served by a two-line HTTP server in this process, so the suite depends on no
 * network and no account. That is also what makes it able to assert the failure path — the server
 * is told to answer 404 and the row has to say so, because a subscription that quietly stopped
 * working looks exactly like a week with no meetings.
 */
import { createServer } from "node:http";
import { chromium } from "playwright";

import { daemon } from "./daemon.mjs";

const problems = [];

// ---- a calendar server ----------------------------------------------------
const stamp = (epoch) => new Date(epoch * 1000).toISOString().replace(/[-:]|\.\d{3}/g, "");
const soon = Math.floor(Date.now() / 1000) + 3600;
const ICS =
  "BEGIN:VCALENDAR\r\nVERSION:2.0\r\n" +
  `BEGIN:VEVENT\r\nUID:e2e-1\r\nSUMMARY:Họp chốt giá\r\nDTSTART:${stamp(soon)}\r\n` +
  `DTEND:${stamp(soon + 1800)}\r\n` +
  "ATTENDEE:mailto:ngoc@acme.vn\r\nATTENDEE:mailto:binh@acme.vn\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

let serving = true;
const calendars = createServer((request, response) => {
  if (!serving) {
    response.writeHead(404).end("gone");
    return;
  }
  response.writeHead(200, { "content-type": "text/calendar" }).end(ICS);
});
await new Promise((resolve) => calendars.listen(0, "127.0.0.1", resolve));
const address = `http://127.0.0.1:${calendars.address().port}/work.ics`;

const engine = await daemon(process.argv, { name: "calendar" });
const browser = await chromium.launch();
const context = await browser.newContext({
  locale: "vi-VN",
  viewport: { width: 1280, height: 950 },
});
const page = await context.newPage();
page.on("pageerror", (e) => problems.push(`pageerror: ${e.message}`));

await page.goto(`${engine.url}?port=${engine.port}&token=${engine.token}#/agenda`, {
  waitUntil: "networkidle",
});

// ---- subscribing ----------------------------------------------------------
{
  await page.getByLabel("Địa chỉ lịch (URL)").fill(address);
  await page.getByLabel("Tên lịch").fill("Lịch công ty");
  await page.getByRole("button", { name: "Đăng ký", exact: true }).click();

  const row = page.getByTestId("calendar-list").getByText("Lịch công ty");
  await row.waitFor({ timeout: 15000 }).catch(() => problems.push("the calendar was never listed"));

  // The meeting from that calendar, on the agenda, without a reload.
  await page
    .getByText("Họp chốt giá")
    .first()
    .waitFor({ timeout: 15000 })
    .catch(() => problems.push("the subscribed calendar's meeting never reached the agenda"));

  const state = await page.getByTestId("calendar-list").innerText();
  if (!/1 sự kiện/.test(state)) problems.push(`the row does not say what it holds: "${state}"`);
}

// ---- a subscription that stops working ------------------------------------
{
  serving = false;
  await page.getByRole("button", { name: /Đồng bộ lại/ }).click();

  const failed = page.getByText(/không tìm thấy lịch/);
  await failed
    .waitFor({ timeout: 15000 })
    .catch(() => problems.push("a broken subscription reports nothing"));

  // And the meetings it already fetched are still there: a laptop that woke up without WiFi should
  // still show this morning's meetings.
  if ((await page.getByText("Họp chốt giá").count()) === 0) {
    problems.push("a failed refresh threw away the calendar it already had");
  }
}

// ---- a URL that is not a calendar -----------------------------------------
{
  await page.getByLabel("Địa chỉ lịch (URL)").fill("file:///etc/passwd");
  await page.getByRole("button", { name: "Đăng ký", exact: true }).click();
  await page
    .getByText(/phải bắt đầu bằng https/)
    .waitFor({ timeout: 10000 })
    .catch(() => problems.push("a file:// URL was not refused in the interface"));
}

await browser.close();
await engine.stop();
calendars.close();

if (problems.length > 0) {
  console.error(problems.map((p) => `  - ${p}`).join("\n"));
  process.exit(1);
}
console.log("calendar ok");
