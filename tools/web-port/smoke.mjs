// Boots the browser package in headless Chromium and checks the engine starts: it serves
// target/web/dist on localhost (a secure context, so WebGPU is exposed), waits for the engine's
// SKATE_WEB_READY (a WebGPU adapter is up) and SKATE_WEB_STOPPED (why startup stopped) console
// lines, and fails on a panic, any console error (a shader the browser's WebGPU rejects, a pipeline
// or validation failure), a page error or a timeout. Saves target/web/smoke.png, but headless
// Chromium does not composite WebGPU canvases into screenshots, so the canvas shows blank there.
//
//   node tools/web-port/smoke.mjs [dist-dir]
//
// Needs Playwright with its Chromium (resolved through NODE_PATH, like a global install).
import { readFile } from 'node:fs/promises';
import { createServer } from 'node:http';
import { createRequire } from 'node:module';
import { extname, join, normalize, resolve } from 'node:path';

const dist = resolve(process.argv[2] ?? 'target/web/dist');
const { chromium } = createRequire(import.meta.url)('playwright');
const types = { '.html': 'text/html', '.js': 'text/javascript', '.wasm': 'application/wasm' };

const server = createServer(async (request, response) => {
  const path = normalize(decodeURIComponent(new URL(request.url, 'http://x').pathname));
  try {
    const body = await readFile(join(dist, path.endsWith('/') ? `${path}index.html` : path));
    response.writeHead(200, { 'content-type': types[extname(path) || '.html'] ?? 'application/octet-stream' });
    response.end(body);
  } catch {
    response.writeHead(404).end();
  }
}).listen(0, 'localhost');
await new Promise((ready) => server.once('listening', ready));
const url = `http://localhost:${server.address().port}/`;

const browser = await chromium.launch({ args: ['--enable-unsafe-webgpu'] });
const page = await browser.newPage({ viewport: { width: 1280, height: 720 } });
const lines = [];
let failure = null;
page.on('console', (message) => {
  const text = message.text();
  lines.push(text);
  if (text.includes('panicked at')) failure ??= `panic: ${text}`;
  else if (message.type() === 'error') failure ??= `console error: ${text}`;
});
page.on('pageerror', (error) => {
  // winit hands control back to the browser's event loop by throwing; that is not a failure.
  if (!String(error).includes('Using exceptions for control flow')) failure ??= `page error: ${error}`;
});

const started = Date.now();
await page.goto(url);
const seen = (tag) => lines.find((line) => line.includes(tag));
while (!failure && !(seen('SKATE_WEB_READY') && seen('SKATE_WEB_STOPPED'))) {
  if (Date.now() - started > 180_000) failure = 'timed out waiting for SKATE_WEB_READY and SKATE_WEB_STOPPED';
  await page.waitForTimeout(250);
}
// A few more frames, so the screenshot shows the first screen drawn.
if (!failure) await page.waitForTimeout(2_000);
await page.screenshot({ path: resolve(dist, '..', 'smoke.png') });
await browser.close();
server.close();

for (const tag of ['SKATE_WEB_READY', 'SKATE_WEB_STOPPED']) if (seen(tag)) console.log(seen(tag));
console.log(`${((Date.now() - started) / 1000).toFixed(1)} s to boot; screenshot: ${resolve(dist, '..', 'smoke.png')}`);
if (failure) {
  console.error(`FAILED: ${failure}`);
  console.error(lines.slice(-20).join('\n'));
  process.exitCode = 1;
}
