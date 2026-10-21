// Assemble the released site (ARCHITECTURE.md section 18): every release's
// bundle at /v/<version>/, never changed, and the latest release at the
// root. The release workflow runs it over every published release.
//
// Usage: node scripts/assemble-site.mjs <releases-dir> <out-dir>
//
// <releases-dir> holds one folder per release, named by its version
// (0.1.0/, 0.2.0/, ...), each an unpacked web bundle. <out-dir> must not
// exist yet.
//
// Every version is served with the latest release's headers, so the latest
// policy has to allow what older versions need. In practice that's one
// thing: each version's HTML report has its own inline style, allowed by its
// hash, so style-src allows the hash of every release's report style. Each
// version's service worker is served uncached, like the root's.
import { cpSync, existsSync, mkdirSync, readdirSync, readFileSync, statSync, writeFileSync } from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

/** A release version (1.2.3) as numbers, for sorting; null if it isn't one. */
export function parseVersion(name) {
  const match = /^(\d+)\.(\d+)\.(\d+)$/.exec(name)
  return match ? match.slice(1).map(Number) : null
}

function compareVersions(a, b) {
  const [x, y] = [parseVersion(a), parseVersion(b)]
  for (let i = 0; i < 3; i++) if (x[i] !== y[i]) return x[i] - y[i]
  return 0
}

/** The 'sha256-...' sources in a _headers file's style-src. */
export function styleHashes(headers) {
  const csp = /^\s*Content-Security-Policy:(.*)$/m.exec(headers)
  const style = csp && /style-src([^;]*)/.exec(csp[1])
  return style ? style[1].match(/'sha256-[A-Za-z0-9+/=]+'/g) ?? [] : []
}

/** The latest release's _headers, allowing every release's report style. */
export function mergeHeaders(latest, all) {
  const hashes = [...new Set(all.flatMap(styleHashes))].sort()
  let found = false
  const merged = latest.replace(/^(\s*Content-Security-Policy:.*?style-src)([^;]*)/m, (_, head, sources) => {
    found = true
    const kept = sources.trim().split(/\s+/).filter((s) => s && !s.startsWith("'sha256-"))
    return `${head} ${[...kept, ...hashes].join(' ')}`
  })
  if (!found) throw new Error("the latest release's _headers has no Content-Security-Policy with a style-src")
  return `${merged.trimEnd()}\n\n/v/:version/sw.js\n  Cache-Control: no-cache\n`
}

/** Assemble the site from the releases in `releasesDir` into `outDir`. */
export function assemble(releasesDir, outDir) {
  const versions = readdirSync(releasesDir)
    .filter((name) => statSync(path.join(releasesDir, name)).isDirectory())
    .sort((a, b) => {
      if (!parseVersion(a) || !parseVersion(b)) throw new Error(`${!parseVersion(a) ? a : b} isn't a release version`)
      return compareVersions(a, b)
    })
  if (!versions.length) throw new Error(`there are no releases in ${releasesDir}`)
  if (existsSync(outDir)) throw new Error(`${outDir} already exists`)
  const latest = versions[versions.length - 1]
  const headers = versions.map((v) => {
    const file = path.join(releasesDir, v, '_headers')
    if (!existsSync(file)) throw new Error(`release ${v} has no _headers`)
    return readFileSync(file, 'utf8')
  })

  // The latest release at the root, then every release under /v/.
  cpSync(path.join(releasesDir, latest), outDir, { recursive: true })
  for (const v of versions) {
    const target = path.join(outDir, 'v', v)
    mkdirSync(target, { recursive: true })
    cpSync(path.join(releasesDir, v), target, {
      recursive: true,
      filter: (source) => path.basename(source) !== '_headers' || path.dirname(source) !== path.join(releasesDir, v),
    })
  }
  writeFileSync(path.join(outDir, '_headers'), mergeHeaders(headers[headers.length - 1], headers))
  return { versions, latest }
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const [releasesDir, outDir] = process.argv.slice(2)
  if (!releasesDir || !outDir) {
    console.error('usage: node scripts/assemble-site.mjs <releases-dir> <out-dir>')
    process.exit(2)
  }
  const { versions, latest } = assemble(releasesDir, outDir)
  console.log(`assembled ${versions.length} release(s) into ${outDir}; ${latest} at the root`)
}
