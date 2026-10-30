// Put the files the web app serves itself (`app_files` in packs/au/pack.toml,
// D-028) into public/data/, so the browser needs no relay for them. Each is
// downloaded from its source once and checked against the pack's SHA-256
// and size; a file already there that matches is kept, and anything else
// there is removed.
//
// Behind a proxy, set NODE_USE_ENV_PROXY=1 so fetch uses HTTPS_PROXY.
import { createHash } from 'node:crypto'
import { existsSync, mkdirSync, readdirSync, readFileSync, renameSync, rmSync, writeFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import path from 'node:path'

const ui = path.dirname(path.dirname(fileURLToPath(import.meta.url)))
const root = path.resolve(ui, '../../..')
const dest = path.join(ui, 'public/data')

// The pack's app files, each with its source's base URL. Just enough TOML
// for the pack's own layout: `[[sources]]` tables, each with a `url`, and
// `[[sources.app_files]]` tables after it.
function appFiles(pack) {
  const files = []
  let url = null
  let current = null
  for (const raw of pack.split('\n')) {
    const line = raw.replace(/#.*$/, '').trim()
    if (line === '[[sources]]') {
      url = null
      current = null
    } else if (line === '[[sources.app_files]]') {
      current = { url }
      files.push(current)
    } else if (line.startsWith('[')) {
      current = null
    } else {
      const m = line.match(/^(\w+)\s*=\s*(?:"([^"]*)"|(\d+))$/)
      if (!m) continue
      const value = m[2] ?? Number(m[3])
      if (current) current[m[1]] = value
      else if (m[1] === 'url') url = value
    }
  }
  return files
}

const sha256 = (data) => createHash('sha256').update(data).digest('hex')

const files = appFiles(readFileSync(path.join(root, 'packs/au/pack.toml'), 'utf8'))
if (files.length === 0) {
  console.error('packs/au/pack.toml lists no app files')
  process.exit(1)
}
mkdirSync(dest, { recursive: true })
// Anything else there, such as a release the pack has moved on from, goes.
const wanted = new Set(files.map((f) => f.name))
for (const name of readdirSync(dest)) {
  if (!wanted.has(name)) {
    rmSync(path.join(dest, name), { force: true })
    console.log(`data ${name}: removed, no longer in the pack`)
  }
}
for (const f of files) {
  if (!f.url || !f.name || !/^[0-9a-f]{64}$/.test(f.sha256 ?? '') || !f.bytes) {
    console.error(`packs/au/pack.toml: incomplete app file ${JSON.stringify(f)}`)
    process.exit(1)
  }
  const target = path.join(dest, f.name)
  if (existsSync(target) && sha256(readFileSync(target)) === f.sha256) {
    console.log(`data ${f.name}: already there`)
    continue
  }
  const from = f.url + f.name
  const response = await fetch(from)
  if (!response.ok) {
    console.error(`${from}: HTTP ${response.status}`)
    process.exit(1)
  }
  const data = Buffer.from(await response.arrayBuffer())
  const actual = sha256(data)
  if (actual !== f.sha256 || data.length !== f.bytes) {
    console.error(`${from}: got ${data.length} bytes with sha256 ${actual}; the pack says ${f.bytes} and ${f.sha256}`)
    process.exit(1)
  }
  writeFileSync(`${target}.part`, data)
  renameSync(`${target}.part`, target)
  console.log(`data ${f.name}: ${data.length} bytes from ${from}`)
}
