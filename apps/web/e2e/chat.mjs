import { chromium } from "playwright";

import { daemon } from "./daemon.mjs";
const engine = await daemon(process.argv, { name: "chat" });
const { url: appUrl, port, token } = engine;
const b = await chromium.launch();
// The suites assert Vietnamese wording, so the browser has to ask for Vietnamese. Without
// this the app honours the machine's locale — which is exactly what it should do, and which made
// every assertion here fail the moment translation landed.
const c = await b.newContext({
  locale: "vi-VN",
  viewport: { width: 1200, height: 820 },
  colorScheme: "dark",
});
const p = await c.newPage();
const problems = [];
p.on("pageerror", (e) => problems.push("pageerror: " + e.message));
await p.goto(`${appUrl}?port=${port}&token=${token}#/chat`, { waitUntil: "networkidle" });
await p.getByRole("heading", { name: "Hỏi kho họp" }).waitFor({ timeout: 10000 });
await p.getByRole("textbox", { name: "Câu hỏi" }).fill("ngân sách");
await p.getByRole("main").getByRole("button", { name: "Hỏi", exact: true }).click();
// Wait for the answer or the failure, rather than for a fixed 2.5 seconds. A refused connection
// to a model that is not running takes about six on this machine, so the fixed wait reported "the
// failure was swallowed" for a message that arrived four seconds later.
let body = "";
for (const deadline = Date.now() + 30_000; Date.now() < deadline;) {
  body = await p.getByRole("main").innerText();
  if (/Ollama|error|lỗi/i.test(body)) break;
  await p.waitForTimeout(250);
}
console.log("shows the question:", body.includes("ngân sách"));
// The question has to match something in the vault. Chat searches first and answers "no meeting
// mentions this" without calling a model at all when nothing does — which is right, and which made
// this assertion fail against a query the seeded vault had never heard of.
//
// With a match found and no model reachable, the failure must reach the user rather than vanish.
const reported = /Ollama|error|lỗi/i.test(body);
console.log("reports the failure:", reported);
if (!reported) problems.push("a failing request produced no visible message");
await p.screenshot({ path: "/tmp/shots/chat.png" });
await b.close();
if (problems.length) {
  console.error(problems.join("\n"));
  process.exit(1);
}
console.log("chat ok");
