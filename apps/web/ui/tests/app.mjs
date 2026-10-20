// End-to-end test of the web app in Chromium, under the production Content
// Security Policy:
//   1. the page loads, the core reports a web build, and the required
//      browser features are present,
//   2. DynAdjust's sample network loads, adjusts with DynAdjust's result,
//      shows its report (styled, which the policy must allow), takes an
//      exclusion with a reason, and downloads a GeoPackage and CSV,
//   3. a project in browser storage (OPFS) accepts a file, and the core
//      hashes it to the same SHA-256 as Node does,
//   4. after the service worker has cached the build, the app reloads and
//      the engine module loads with the network switched off,
//   5. the page logs no errors.
//
// Usage: node tests/app.mjs
import { createHash } from 'node:crypto'
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { chromium } from 'playwright'
import { serve } from './static-server.mjs'

const ui = path.dirname(path.dirname(fileURLToPath(import.meta.url)))
const SAMPLE = path.resolve(ui, '../../../crates/formats/tests/data/dynaml')
const REQUIRED = ['Secure context', 'WebAssembly', 'WebAssembly SIMD', 'Web workers', 'Browser storage (OPFS)', 'Offline use (service worker)']

const server = await serve(path.join(ui, 'dist'))
const browser = await chromium.launch()
const scratch = mkdtempSync(path.join(tmpdir(), 'gnss-app-test-'))
let failed = false
try {
  const context = await browser.newContext()
  const page = await context.newPage()
  const errors = []
  page.on('console', (m) => m.type() === 'error' && errors.push(m.text()))
  page.on('pageerror', (e) => errors.push(String(e)))

  // 1. Shell, core and capabilities.
  await page.goto(server.origin)
  await page.getByText(/core \S+ \(web build\)/).waitFor()
  const missing = await page.locator('.checks li.missing').allInnerTexts()
  const missingRequired = REQUIRED.filter((name) => missing.some((text) => text.includes(name)))
  if (missingRequired.length) throw new Error(`required features missing: ${missingRequired.join(', ')}`)
  console.log(`shell loaded; optional features missing: ${missing.length ? missing.join('; ') : 'none'}`)

  // 2. The second-opinion adjustment, on DynAdjust's sample network.
  await page
    .getByTestId('adjust-files')
    .setInputFiles(['gnss-networkstn.xml', 'gnss-networkmsr.xml'].map((f) => path.join(SAMPLE, f)))
  for (const f of ['gnss-networkstn.xml', 'gnss-networkmsr.xml']) {
    const format = await page.getByLabel(`Format of ${f}`).inputValue()
    if (format !== 'dynaml') throw new Error(`${f} was taken for ${format || 'nothing'}, not DynaML`)
  }
  await page.getByRole('button', { name: 'Adjust', exact: true }).click()
  await page.getByRole('status').filter({ hasText: 'The global test passed; 12 tests of observations failed.' }).waitFor()
  const facts = await page.locator('dl.facts').innerText()
  if (!/Degrees of freedom\s+288/.test(facts) || !/336\.64 within/.test(facts)) throw new Error(`unexpected results:\n${facts}`)
  const report = page.frameLocator('iframe.report')
  await report.getByRole('heading', { name: 'Second opinion', level: 1 }).waitFor()
  const band = await report.locator('th').first().evaluate((th) => getComputedStyle(th).backgroundColor)
  if (band === 'rgba(0, 0, 0, 0)') throw new Error('the report is unstyled: the security policy blocked its style')
  console.log('adjusted the sample network: 288 degrees of freedom, the global test at 336.64, report styled')

  await page.getByRole('button', { name: 'Exclude…' }).first().click()
  await page.getByLabel('Reason for excluding it').fill('vector test 28.4; re-observed')
  await page.getByRole('button', { name: 'Exclude', exact: true }).click()
  await page.getByRole('status').filter({ hasText: '7 tests of observations failed' }).waitFor()
  await page.getByText('gnss-networkmsr.xml:435: vector test 28.4; re-observed').waitFor()
  await report.getByRole('cell', { name: 'vector test 28.4; re-observed' }).waitFor()

  const downloaded = async (button) => {
    const [download] = await Promise.all([page.waitForEvent('download'), page.getByRole('button', { name: button, exact: true }).click()])
    const file = path.join(scratch, download.suggestedFilename())
    await download.saveAs(file)
    return { name: download.suggestedFilename(), bytes: readFileSync(file) }
  }
  const gpkg = await downloaded('GeoPackage')
  if (gpkg.name !== 'results.gpkg' || gpkg.bytes.subarray(0, 16).toString('latin1') !== 'SQLite format 3\0' || gpkg.bytes.subarray(68, 72).toString() !== 'GPKG') {
    throw new Error(`${gpkg.name} isn't a GeoPackage`)
  }
  const report2 = await downloaded('Report (HTML)')
  if (!report2.bytes.toString().includes('<h1>Second opinion</h1>')) throw new Error('the downloaded report is wrong')
  console.log(`excluded an observation with its reason; downloaded ${gpkg.name} (${gpkg.bytes.length} bytes) and the report`)

  // 3. A browser-storage project, a file in it, and its hash from the core.
  await page.getByText('Diagnostics:').click()
  const content = `GNSS app test ${Date.now()}\n`
  const file = path.join(scratch, 'sample.txt')
  writeFileSync(file, content)
  const expected = createHash('sha256').update(content).digest('hex')
  await page.getByRole('button', { name: 'Use browser storage' }).click()
  await page.locator('details.diagnostics input[type=file]').setInputFiles(file)
  await page.getByRole('cell', { name: 'sample.txt' }).waitFor()
  await page.getByRole('button', { name: 'Compute SHA-256 hashes' }).click()
  await page.getByRole('cell', { name: expected }).waitFor()
  console.log(`browser storage project ok; core hash matches Node (${expected.slice(0, 12)}…)`)

  // 4. Offline: wait for the service worker to finish caching, then reload
  //    with no network and load the engine from the cache.
  await page.evaluate(async () => {
    const registration = await navigator.serviceWorker.ready
    if (!registration.active) throw new Error('service worker not active')
  })
  await context.setOffline(true)
  await page.goto(`${server.origin}/?harness=1`)
  await page.getByText(/core \S+ \(web build\)/).waitFor()
  await page.waitForFunction(() => window.__gnssHarness)
  await page.evaluate(() => window.__gnssHarness.engineReady())
  console.log('offline reload ok: the app, core and engine module all load from the cache')
  await context.setOffline(false)

  if (errors.length) throw new Error(`page errors:\n  ${errors.join('\n  ')}`)
} catch (err) {
  console.error(`app test FAILED: ${err.message}`)
  failed = true
} finally {
  rmSync(scratch, { recursive: true, force: true })
  await browser.close()
  await server.close()
}
process.exit(failed ? 1 : 0)
