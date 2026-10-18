// Run the spike baselines through the web app's engine workers in headless
// Chromium, and append the results to work/results/runs.jsonl as build
// "browser".
//
// The built web app (apps/web/ui/dist) is served with the same Content
// Security Policy as production, so this also checks the app works under it.
// Inputs come from work/runs/, prepared by run.py, under the same names the
// native runs used, so solution files can be compared byte for byte.
//
// Usage (after building the web app and running run.py once to prepare inputs):
//   node browser/run_browser.mjs [--only PAIR,...] [--cases CASE,...]
//                                [--log FILE] [--repeat N] [--trimmed]
import { createHash } from 'node:crypto'
import { appendFileSync, readFileSync } from 'node:fs'
import { createRequire } from 'node:module'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const here = path.dirname(fileURLToPath(import.meta.url))
const spike = path.dirname(here)
const ui = path.resolve(spike, '../../apps/web/ui')
// Use the web app's pinned Playwright and its static test server.
const { chromium } = createRequire(path.join(ui, 'package.json'))('playwright')
const { serve } = await import(path.join(ui, 'tests/static-server.mjs'))

const CASES = {
  '2h-combined': ['02:00:00', '04:00:00'],
  '2h-forward': ['02:00:00', '04:00:00'],
  '2h-backward': ['02:00:00', '04:00:00'],
  '24h-combined': null,
}

const args = process.argv.slice(2)
const opt = (name) => {
  const i = args.indexOf(name)
  return i >= 0 ? args[i + 1].split(',') : null
}
const only = opt('--only')
const cases = opt('--cases') ?? Object.keys(CASES)
const logName = opt('--log')?.[0] ?? 'runs.jsonl'
const repeat = Number(opt('--repeat')?.[0] ?? 1)
// Session-trimmed inputs from trim_rinex.py, for the timing pass.
const trimmed = args.includes('--trimmed')

const server = await serve(path.join(ui, 'dist'), { 'spike-data': path.join(spike, 'work/runs') })
const origin = server.origin

const browser = await chromium.launch()
const page = await browser.newPage()
const problems = []
page.on('console', (m) => m.type() === 'error' && problems.push(m.text()))
page.on('pageerror', (e) => problems.push(String(e)))
await page.goto(`${origin}/?harness=1`)
await page.waitForFunction(() => window.__gnssHarness)
const info = await page.evaluate(() => window.__gnssHarness.coreInfo())
await page.evaluate(() => window.__gnssHarness.engineReady())
const browserVersion = browser.version()
console.log(`core ${info.version} (${info.build}), Chromium ${browserVersion}`)

const manifest = JSON.parse(readFileSync(path.join(spike, 'manifest.json'), 'utf8'))
const date = manifest.date.replaceAll('-', '/')
const log = path.join(spike, 'work/results', logName)

for (const b of manifest.baselines) {
  const pair = `${b.base}-${b.rover}`
  if (only && !only.includes(pair)) continue
  for (const kase of cases) for (let rep = 0; rep < repeat; rep++) {
    const window = CASES[kase]
    const engineArgs = ['-k', `${kase}.conf`]
    if (window) engineArgs.push('-ts', date, window[0], '-te', date, window[1])
    engineArgs.push('-o', 'out.pos', 'rover.obs', 'base.obs', 'nav.rnx')
    const inputs = ['rover.obs', 'base.obs', 'nav.rnx', 'igs14.atx', `${kase}.conf`].map((name) => ({
      name,
      url: `${origin}/spike-data/${pair}/${trimmed ? 'trim/' : ''}${name}`,
    }))
    const r = await page.evaluate(
      (request) => window.__gnssHarness.runEngine(request),
      { args: engineArgs, inputs, outputs: ['out.pos'] },
    )
    const solution = r.outputs['out.pos']
    const sha256 = solution === undefined ? null : createHash('sha256').update(solution, 'utf8').digest('hex')
    const record = {
      pair,
      length_km: b.length_km,
      case: trimmed ? `${kase}-trimmed` : kase,
      build: 'browser',
      rep,
      browser: `Chromium ${browserVersion}`,
      exit: r.exitCode,
      wall_s: Math.round(r.wallMs) / 1000,
      module_memory_mb: Math.round((r.memoryBytes / 2 ** 20) * 10) / 10,
      sha256,
      error: r.error ?? null,
      stderr_tail: r.stderrTail.slice(-300),
    }
    appendFileSync(log, JSON.stringify(record) + '\n')
    console.log(
      `${pair.padEnd(10)} ${kase.padEnd(13)} browser      exit=${r.exitCode} ${record.wall_s.toFixed(2).padStart(7)}s ` +
        `${String(record.module_memory_mb).padStart(7)} MB ${sha256?.slice(0, 12)}`,
    )
  }
}

if (problems.length) console.log(`page errors:\n  ${problems.join('\n  ')}`)
await browser.close()
await server.close()
