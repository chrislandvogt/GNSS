// End-to-end smoke test of the web app in a browser engine, under the
// production Content Security Policy:
//   1. the core loads in its worker and reports a web build,
//   2. the engine workers download, verify and compile the RTKLIB-EX module,
//   3. the engine reproduces the golden output for RTKLIB's own test case
//      (engine-modules/rtklib-ex/golden.sha256) byte for byte,
//   4. convbin converts RTKLIB's u-blox sample to the golden RINEX
//      (golden-convbin.sha256, leaving out the record with the run's time),
//   5. the page logs no errors.
//
// Usage: [BROWSER=chromium|firefox|webkit] node tests/engine-smoke.mjs <RTKLIB test/data/rinex directory>
//
// Run it in all three engines to check that WebAssembly gives byte-identical
// engine output in every browser (D-023).
import { createHash } from 'node:crypto'
import { readFileSync } from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { chromium, firefox, webkit } from 'playwright'
import { serve } from './static-server.mjs'

const ui = path.dirname(path.dirname(fileURLToPath(import.meta.url)))
const testData = process.argv[2]
if (!testData) {
  console.error('usage: node tests/engine-smoke.mjs <rtklib test/data/rinex directory>')
  process.exit(2)
}
const golden = readFileSync(path.resolve(ui, '../../../engine-modules/rtklib-ex/golden.sha256'), 'utf8')
  .split('\n')
  .find((line) => line.endsWith(' test4.pos'))
  .split(/\s+/)[0]

const engines = { chromium, firefox, webkit }
const engineName = process.env.BROWSER ?? 'chromium'
if (!engines[engineName]) {
  console.error(`unknown BROWSER '${engineName}': use chromium, firefox or webkit`)
  process.exit(2)
}

let browser
try {
  browser = await engines[engineName].launch()
} catch (err) {
  console.error(`couldn't launch ${engineName} (install it with: npx playwright install ${engineName})\n${err.message}`)
  process.exit(2)
}
const server = await serve(path.join(ui, 'dist'), { 'test-data': testData })
let failed = false
try {
  const page = await browser.newPage()
  const errors = []
  page.on('console', (m) => m.type() === 'error' && errors.push(m.text()))
  page.on('pageerror', (e) => errors.push(String(e)))

  await page.goto(`${server.origin}/?harness=1`)
  await page.waitForFunction(() => window.__gnssHarness)
  const info = await page.evaluate(() => window.__gnssHarness.coreInfo())
  console.log(`core ${info.version} (${info.build}) in ${engineName} ${browser.version()}`)
  if (info.build !== 'web') throw new Error(`expected the web build, got ${info.build}`)

  await page.evaluate(() => window.__gnssHarness.engineReady())
  const files = ['07590920.05o', '30400920.05n', '30400920.05o']
  const result = await page.evaluate(
    (request) => window.__gnssHarness.runEngine(request),
    {
      args: ['-t', '-p', '3', '-e', '-r', '-3978241.958', '3382840.234', '3649900.853', ...files],
      inputs: files.map((name) => ({ name, url: `${server.origin}/test-data/${name}` })),
      outputs: [],
    },
  )
  const actual = createHash('sha256').update(result.stdout, 'utf8').digest('hex')
  if (result.exitCode !== 0) throw new Error(`engine exited with ${result.exitCode}: ${result.error ?? result.stderrTail}`)
  if (actual !== golden) throw new Error(`test4.pos sha256 ${actual}, expected ${golden}`)
  console.log(`engine output matches the golden test4.pos (${actual})`)

  const goldenConvbin = Object.fromEntries(
    readFileSync(path.resolve(ui, '../../../engine-modules/rtklib-ex/golden-convbin.sha256'), 'utf8')
      .trim()
      .split('\n')
      .map((line) => line.split(/\s+/).reverse()),
  )
  const raw = 'ubx_20080526.ubx'
  const converted = await page.evaluate(
    (request) => window.__gnssHarness.runConverter(request),
    {
      args: ['-os', '-o', 'out.obs', '-n', 'out.nav', raw],
      inputs: [{ name: raw, url: `${server.origin}/test-data/${raw}` }],
      outputs: ['out.obs', 'out.nav'],
    },
  )
  if (converted.exitCode !== 0) throw new Error(`convbin exited with ${converted.exitCode}: ${converted.error ?? converted.stderrTail}`)
  for (const kind of ['obs', 'nav']) {
    const text = converted.outputs[`out.${kind}`] ?? ''
    const kept = text.split('\n').filter((line) => !line.includes('PGM / RUN BY / DATE')).join('\n')
    const hash = createHash('sha256').update(kept, 'utf8').digest('hex')
    const expected = goldenConvbin[`ubx_20080526.${kind}`]
    if (hash !== expected) throw new Error(`convbin ubx_20080526.${kind} sha256 ${hash}, expected ${expected}`)
  }
  console.log('convbin output matches the golden RINEX for the u-blox sample')
  if (errors.length) throw new Error(`page errors:\n  ${errors.join('\n  ')}`)
} catch (err) {
  console.error(`engine smoke test FAILED: ${err.message}`)
  failed = true
} finally {
  await browser.close()
  await server.close()
}
process.exit(failed ? 1 : 0)
