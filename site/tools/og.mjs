// Renders tools/og/og.html at 1200×630 with headless Chrome over the DevTools
// Protocol and writes public/assets/og.jpg. Node 24+ (built-in WebSocket and
// fetch), no npm packages. Serves the site/ directory itself so the card can
// use the real fonts and photos by relative path.
//
//   node tools/og.mjs            # → public/assets/og.jpg
//   CHROME=/path/to/chrome node tools/og.mjs
import { createServer } from 'node:http';
import { readFile, writeFile, mkdtemp, rm } from 'node:fs/promises';
import { spawn } from 'node:child_process';
import { extname, join, resolve, dirname } from 'node:path';
import { tmpdir } from 'node:os';
import { fileURLToPath } from 'node:url';

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const CHROME = process.env.CHROME || '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';
const OUT = join(ROOT, 'public/assets/og.jpg');
const TYPES = { '.html': 'text/html; charset=utf-8', '.css': 'text/css', '.woff2': 'font/woff2', '.webp': 'image/webp', '.png': 'image/png', '.jpg': 'image/jpeg', '.svg': 'image/svg+xml' };
const sleep = ms => new Promise(r => setTimeout(r, ms));

const server = createServer(async (req, res) => {
  try {
    const path = resolve(ROOT, '.' + decodeURIComponent(new URL(req.url, 'http://x').pathname));
    if (!path.startsWith(ROOT + '/')) throw new Error('outside root');
    const body = await readFile(path);
    res.writeHead(200, { 'content-type': TYPES[extname(path)] || 'application/octet-stream' });
    res.end(body);
  } catch { res.writeHead(404); res.end(); }
});
await new Promise(r => server.listen(0, '127.0.0.1', r));
const url = `http://127.0.0.1:${server.address().port}/tools/og/og.html`;

const profile = await mkdtemp(join(tmpdir(), 'owt-og-'));
const chrome = spawn(CHROME, ['--headless=new', '--remote-debugging-port=0', `--user-data-dir=${profile}`,
  '--no-first-run', '--hide-scrollbars', '--force-device-scale-factor=1', 'about:blank'], { stdio: 'ignore' });
let port = 0;
for (let i = 0; i < 100 && !port; i++) {
  try { port = Number((await readFile(join(profile, 'DevToolsActivePort'), 'utf8')).split('\n')[0]); } catch { await sleep(100); }
}
if (!port) throw new Error('Chrome did not start');
const targets = await (await fetch(`http://127.0.0.1:${port}/json`)).json();
const ws = new WebSocket(targets.find(t => t.type === 'page').webSocketDebuggerUrl);
await new Promise(r => ws.onopen = r);
let id = 0; const pending = new Map();
ws.onmessage = e => { const m = JSON.parse(e.data); if (m.id && pending.has(m.id)) { pending.get(m.id)(m); pending.delete(m.id); } };
const send = (method, params = {}) => new Promise(res => { const i = ++id; pending.set(i, res); ws.send(JSON.stringify({ id: i, method, params })); });

await send('Page.enable'); await send('Runtime.enable');
await send('Emulation.setDeviceMetricsOverride', { width: 1200, height: 630, deviceScaleFactor: 1, mobile: false });
await send('Page.navigate', { url });
await sleep(800);
const ready = await send('Runtime.evaluate', {
  expression: `Promise.all([document.fonts.ready, ...[...document.images].map(i => i.decode())]).then(() => document.fonts.status + ' ' + document.images.length + ' images')`,
  awaitPromise: true, returnByValue: true,
});
await sleep(300);
const shot = await send('Page.captureScreenshot', { format: 'jpeg', quality: 90, clip: { x: 0, y: 0, width: 1200, height: 630, scale: 1 } });
await writeFile(OUT, Buffer.from(shot.result.data, 'base64'));
console.log(OUT, '|', ready.result.result.value);
ws.close(); chrome.kill(); server.close();
await sleep(300); await rm(profile, { recursive: true, force: true });
process.exit(0);
