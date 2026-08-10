import { chromium } from 'playwright';
const [, , appUrl, port, token] = process.argv;
const b = await chromium.launch();
const c = await b.newContext({ viewport: { width: 1200, height: 820 }, colorScheme: 'dark' });
const p = await c.newPage();
const problems = [];
p.on('pageerror', (e) => problems.push('pageerror: ' + e.message));
await p.goto(`${appUrl}?port=${port}&token=${token}#/chat`, { waitUntil: 'networkidle' });
await p.getByRole('heading', { name: 'Hỏi kho họp' }).waitFor({ timeout: 10000 });
await p.getByRole('textbox', { name: 'Câu hỏi' }).fill('ngân sách quý bốn');
await p.getByRole('main').getByRole('button', { name: 'Hỏi', exact: true }).click();
await p.waitForTimeout(2500);
const body = await p.getByRole('main').innerText();
console.log('shows the question:', body.includes('ngân sách quý bốn'));
// With no model reachable the failure must reach the user, not vanish.
const reported = /Ollama|error|lỗi/i.test(body);
console.log('reports the failure:', reported);
if (!reported) problems.push('a failing request produced no visible message');
await p.screenshot({ path: '/tmp/shots/chat.png' });
await b.close();
if (problems.length) { console.error(problems.join('\n')); process.exit(1); }
console.log('chat ok');
