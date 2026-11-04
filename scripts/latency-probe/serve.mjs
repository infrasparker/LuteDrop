// Dev-only static server for the latency probe page.
//
// Why a server at all: the probe page needs to read the broadcast stream with
// fetch() and a Web Audio graph. Serving the stream from our own origin (via the
// /stream proxy below) means the browser never applies cross-origin rules to it.
//
// Usage: node scripts/latency-probe/serve.mjs --stream http://127.0.0.1:5883/listen/<key> [--port 8790] [--fail] [--no-cors]
import http from 'node:http';
import https from 'node:https';
import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const here = path.dirname(fileURLToPath(import.meta.url));
const moduleDir = path.resolve(here, '../../foundry-module/lutedrop-stream/scripts');

function arg(name, fallback) {
  const i = process.argv.indexOf(`--${name}`);
  return i !== -1 && process.argv[i + 1] ? process.argv[i + 1] : fallback;
}

const streamUrl = arg('stream');
const port = Number(arg('port', '8790'));
// Test switches for the WebCodecs player's reconnect and give-up paths (the page is
// same-origin, so --no-cors only matters when the stream is fetched from another
// origin): --no-cors leaves out the CORS header (fetch is refused -> give-up after 3),
// --fail answers 404 on /stream (the server answered -> retry forever, no give-up).
const noCors = process.argv.includes('--no-cors');
const fail = process.argv.includes('--fail');
if (!streamUrl) {
  console.error('Usage: node serve.mjs --stream http://127.0.0.1:5883/listen/<key> [--port 8790] [--fail] [--no-cors]');
  process.exit(1);
}

async function sendFile(res, file, type) {
  try {
    const body = await readFile(file);
    res.writeHead(200, { 'Content-Type': type, 'Cache-Control': 'no-store' });
    res.end(body);
  } catch {
    res.writeHead(404).end('not found');
  }
}

// Pipes the upstream stream straight through. Nothing is buffered here, because
// buffering would add the very delay we are trying to measure.
function proxyStream(req, res) {
  const client = streamUrl.startsWith('https:') ? https : http;
  const upstream = client.request(streamUrl, (up) => {
    res.writeHead(up.statusCode ?? 502, {
      'Content-Type': up.headers['content-type'] ?? 'application/octet-stream',
      ...(noCors ? {} : { 'Access-Control-Allow-Origin': '*' }),
      'Cache-Control': 'no-store',
    });
    up.pipe(res);
    // If LuteDrop dies mid-stream, pipe() would leave the response open forever.
    up.on('error', () => res.destroy());
    up.on('aborted', () => res.destroy());
  });
  upstream.on('error', (e) => {
    if (!res.headersSent) res.writeHead(502);
    res.end(`upstream error: ${e.message}`);
  });
  // If the page goes away, stop pulling audio from LuteDrop too.
  res.on('close', () => upstream.destroy());
  upstream.end();
}

http
  .createServer((req, res) => {
    const { pathname } = new URL(req.url, 'http://localhost');
    if (pathname === '/') return sendFile(res, path.join(here, 'index.html'), 'text/html');
    if (pathname === '/stream') return fail ? res.writeHead(404).end('not found (--fail)') : proxyStream(req, res);
    if (pathname.startsWith('/module/')) {
      // basename() blocks "../" tricks; the module folder is flat anyway.
      const name = path.basename(pathname);
      return sendFile(res, path.join(moduleDir, name), 'text/javascript');
    }
    res.writeHead(404).end('not found');
  })
  .listen(port, '127.0.0.1', () => {
    console.log(`Latency probe: open http://127.0.0.1:${port}/`);
    console.log(`Proxying /stream -> ${streamUrl}`);
  });
