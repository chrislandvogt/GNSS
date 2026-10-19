// Check the licences of every package in package-lock.json against the
// project's policy (D-005): permissive licences only, no GPL, AGPL or LGPL.
import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import path from 'node:path'

const ALLOWED = new Set([
  'MIT', 'ISC', 'Apache-2.0', 'BSD-2-Clause', 'BSD-3-Clause', '0BSD',
  'CC0-1.0', 'CC-BY-4.0', 'MPL-2.0', 'Unlicense', 'BlueOak-1.0.0', 'Python-2.0',
])

// SPDX expressions such as "(MIT OR Apache-2.0)": OR needs one allowed term,
// AND needs all of them.
function allowed(expr) {
  const e = expr.trim().replace(/^\((.*)\)$/, '$1')
  if (/\sOR\s/.test(e)) return e.split(/\sOR\s/).some(allowed)
  if (/\sAND\s/.test(e)) return e.split(/\sAND\s/).every(allowed)
  return ALLOWED.has(e)
}

const ui = path.dirname(path.dirname(fileURLToPath(import.meta.url)))
const lock = JSON.parse(readFileSync(path.join(ui, 'package-lock.json'), 'utf8'))
const problems = []
let count = 0
for (const [name, pkg] of Object.entries(lock.packages)) {
  if (name === '') continue
  count++
  const licence = pkg.license
  if (!licence || !allowed(licence)) problems.push(`${name}: ${licence ?? 'no licence declared'}`)
}
if (problems.length) {
  console.error(`Packages outside the licence policy (D-005):\n  ${problems.join('\n  ')}`)
  process.exit(1)
}
console.log(`licences ok: ${count} packages`)
