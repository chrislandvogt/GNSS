// A minimal static server for tests: serves the built app (dist/) with the
// production response headers from dist/_headers, so tests run under the
// same Content Security Policy as users. Extra directories can be mounted
// under URL prefixes, for test data.
import { createReadStream, existsSync, readFileSync, statSync } from 'node:fs'
import http from 'node:http'
import path from 'node:path'

const TYPES = {
  '.html': 'text/html',
  '.js': 'text/javascript',
  '.css': 'text/css',
  '.json': 'application/json',
  '.wasm': 'application/wasm',
}

/** Headers that dist/_headers applies to every path. */
function productionHeaders(dist) {
  const text = readFileSync(path.join(dist, '_headers'), 'utf8')
  const all = text.split(/^\/sw\.js$/m)[0]
  return Object.fromEntries(
    all
      .split('\n')
      .filter((line) => /^\s+[A-Za-z-]+:/.test(line))
      .map((line) => {
        const [key, ...value] = line.trim().split(':')
        return [key, value.join(':').trim()]
      }),
  )
}

/**
 * Serve `dist` at / and each `mounts[prefix]` directory at /prefix/.
 * Resolves to { origin, close }.
 */
export async function serve(dist, mounts = {}) {
  const headers = productionHeaders(dist)
  const server = http.createServer((req, res) => {
    const pathname = decodeURIComponent(new URL(req.url, 'http://localhost').pathname)
    const prefix = Object.keys(mounts).find((p) => pathname.startsWith(`/${p}/`))
    const [root, rel] = prefix
      ? [path.resolve(mounts[prefix]), pathname.slice(prefix.length + 2)]
      : [path.resolve(dist), pathname === '/' ? 'index.html' : pathname.slice(1)]
    const file = path.resolve(root, rel)
    if (!file.startsWith(root + path.sep) || !existsSync(file) || statSync(file).isDirectory()) {
      res.writeHead(404).end()
      return
    }
    res.writeHead(200, {
      ...headers,
      'Content-Type': TYPES[path.extname(file)] ?? 'application/octet-stream',
      'Content-Length': statSync(file).size,
    })
    createReadStream(file).pipe(res)
  })
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve))
  return {
    origin: `http://127.0.0.1:${server.address().port}`,
    close: () => new Promise((resolve) => server.close(resolve)),
  }
}
