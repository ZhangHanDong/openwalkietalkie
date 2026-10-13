// Section screenshots of the local site over the Chrome DevTools Protocol.
// Headless Chrome's --screenshot flag cannot scroll, so this drives a page
// instead. Node 24+ (built-in WebSocket), no npm packages.
//
//   python3 -m http.server 8787 -d public &
//   "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" --headless=new \
//       --remote-debugging-port=9333 --user-data-dir=/tmp/owt-shoot about:blank &
//   node tools/shoot.mjs out/            # → out/<section>-<scheme>-<width>.png
import { writeFileSync, mkdirSync } from 'node:fs';
const PORT = 9333, BASE = process.env.SITE || 'http://127.0.0.1:8787/';
const out = (process.argv[2] || 'shots').replace(/\/?$/, '/');
mkdirSync(out, { recursive: true });
const jobs = [];
for (const s of ['top', 'pile', 'features', 'demo', 'works', 'faces', 'blueprint', 'real', 'footer']) jobs.push([s, 'light', 1400]);
for (const s of ['top', 'demo', 'faces', 'real']) jobs.push([s, 'dark', 1400]);
for (const s of ['top', 'demo', 'faces']) jobs.push([s, 'light', 390]);
const targets = await (await fetch(`http://127.0.0.1:${PORT}/json`)).json();
const page = targets.find(t => t.type === 'page');
const ws = new WebSocket(page.webSocketDebuggerUrl);
await new Promise(r => ws.onopen = r);
let id = 0; const pending = new Map();
ws.onmessage = e => { const m = JSON.parse(e.data); if (m.id && pending.has(m.id)) { pending.get(m.id)(m); pending.delete(m.id); } };
const send = (method, params = {}) => new Promise(res => { const i = ++id; pending.set(i, res); ws.send(JSON.stringify({ id: i, method, params })); });
const sleep = ms => new Promise(r => setTimeout(r, ms));
await send('Page.enable'); await send('Runtime.enable');
for (const [sect, scheme, width] of jobs) {
  await send('Emulation.setDeviceMetricsOverride', { width, height: width < 600 ? 844 : 1000, deviceScaleFactor: 1, mobile: width < 600 });
  await send('Emulation.setEmulatedMedia', { features: [{ name: 'prefers-color-scheme', value: scheme }] });
  await send('Runtime.evaluate', { expression: 'try{localStorage.removeItem("owt-theme")}catch(e){}' });
  await send('Page.navigate', { url: BASE + '?' + sect + scheme });
  await sleep(2500);
  const expr = `document.documentElement.style.scrollBehavior='auto';document.querySelectorAll('.reveal').forEach(e=>e.classList.add('revealed'));` +
    (sect === 'top' ? 'window.scrollTo(0,0);' : `var el=document.getElementById('${sect}'); if(el){el.scrollIntoView(); window.scrollBy(0,-${sect === 'footer' ? 0 : 40});}`) +
    `[document.documentElement.getAttribute('data-theme'), window.scrollY, document.documentElement.scrollWidth, window.innerWidth].join(' ')`;
  const r = await send('Runtime.evaluate', { expression: expr, returnByValue: true });
  await sleep(900);
  const shot = await send('Page.captureScreenshot', { format: 'png' });
  const name = `${out}${sect}-${scheme}-${width}.png`;
  writeFileSync(name, Buffer.from(shot.result.data, 'base64'));
  console.log(name, '| theme, scrollY, scrollWidth, innerWidth =', r.result.result.value);
}
ws.close(); process.exit(0);
