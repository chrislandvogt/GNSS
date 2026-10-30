import { svelte } from '@sveltejs/vite-plugin-svelte'
import { createHash } from 'node:crypto'
import { readdirSync, readFileSync, statSync, writeFileSync } from 'node:fs'
import path from 'node:path'
import { defineConfig, type Plugin } from 'vite'

// Files the service worker shouldn't cache: itself, and host configuration.
const NOT_CACHED = new Set(['sw.js', '_headers'])

function listFiles(dir: string, prefix = ''): string[] {
  return readdirSync(dir).flatMap((name) => {
    const full = path.join(dir, name)
    const rel = prefix ? `${prefix}/${name}` : name
    return statSync(full).isDirectory() ? listFiles(full, rel) : [rel]
  })
}

// After the build, write the list of files into sw.js, with a version derived
// from their contents. Each build therefore gets its own cache, and the
// browser sees a changed service worker whenever any file changes
// (ARCHITECTURE.md section 14.3).
function precacheManifest(): Plugin {
  let outDir = 'dist'
  return {
    name: 'gnss-precache-manifest',
    apply: 'build',
    configResolved(config) {
      outDir = path.resolve(config.root, config.build.outDir)
    },
    closeBundle() {
      // Nor the data files (data/, D-028): they're large, and the fetcher
      // keeps what it takes from them in the download cache.
      const files = listFiles(outDir)
        .filter((f) => !NOT_CACHED.has(f) && !f.startsWith('data/'))
        .sort()
      const hash = createHash('sha256')
      for (const f of files) hash.update(f).update(readFileSync(path.join(outDir, f)))
      const manifest = { version: hash.digest('hex').slice(0, 16), files }
      const swPath = path.join(outDir, 'sw.js')
      const sw = readFileSync(swPath, 'utf8')
      writeFileSync(swPath, sw.replace('"__PRECACHE_MANIFEST__"', JSON.stringify(manifest)))
    },
  }
}

export default defineConfig({
  // Relative URLs, so one build works at any path: the site root, a
  // versioned /v/<version>/ path, or `gnss serve` on localhost.
  base: './',
  plugins: [svelte(), precacheManifest()],
  build: {
    target: 'es2023',
    // Keep the WebAssembly files as separate, cacheable assets.
    assetsInlineLimit: 0,
  },
  worker: {
    format: 'es',
  },
})
