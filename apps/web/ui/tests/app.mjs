// End-to-end test of the web app shell in Chromium, under the production
// Content Security Policy:
//   1. the page loads, the core reports a web build, and the required
//      browser features are present,
//   2. a project in browser storage (OPFS) accepts a file, and the core
//      hashes it to the same SHA-256 as Node does,
//   3. after the service worker has cached the build, the app reloads and
//      the engine module loads with the network switched off,
//   4. the page logs no errors.
//
// Usage: node tests/app.mjs
import { createHash } from 'node:crypto'
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { chromium } from 'playwright'
import { serve } from './static-server.mjs'

const ui = path.dirname(path.dirname(fileURLToPath(import.meta.url)))
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

  // 2. A browser-storage project, a file in it, and its hash from the core.
  const content = `GNSS app test ${Date.now()}\n`
  const file = path.join(scratch, 'sample.txt')
  writeFileSync(file, content)
  const expected = createHash('sha256').update(content).digest('hex')
  await page.getByRole('button', { name: 'Use browser storage' }).click()
  await page.locator('input[type=file]').setInputFiles(file)
  await page.getByRole('cell', { name: 'sample.txt' }).waitFor()
  await page.getByRole('button', { name: 'Compute SHA-256 hashes' }).click()
  await page.getByRole('cell', { name: expected }).waitFor()
  console.log(`browser storage project ok; core hash matches Node (${expected.slice(0, 12)}…)`)

  // 3. Offline: wait for the service worker to finish caching, then reload
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
