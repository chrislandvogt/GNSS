// End-to-end test of the web app in Chromium, under the production Content
// Security Policy:
//   1. the page loads, the core reports a web build, and the required
//      browser features are present,
//   2. DynAdjust's sample network loads, adjusts with DynAdjust's result,
//      shows its report (styled, which the policy must allow), takes an
//      exclusion with a reason, and downloads a GeoPackage and CSV,
//   3. a project in browser storage (OPFS) accepts a file, and the core
//      hashes it to the same SHA-256 as Node does; a project zip imports into
//      browser storage, is checked with its problem reported by file,
//      exports without its cache, can't be imported over itself, and is
//      removed,
//   4. after the service worker has cached the build, the app reloads and
//      the engine module loads with the network switched off,
//   5. the page logs no errors.
//
// Usage: node tests/app.mjs
import { createHash } from 'node:crypto'
import { crc32, gzipSync } from 'node:zlib'
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { chromium } from 'playwright'
import { serve } from './static-server.mjs'

const ui = path.dirname(path.dirname(fileURLToPath(import.meta.url)))
const SAMPLE = path.resolve(ui, '../../../crates/formats/tests/data/dynaml')
const RINEX = path.resolve(ui, '../../../crates/rinex-io/tests/data')
// RTKLIB's u-blox sample, copied there by CI's engine-module job.
const UBX = path.resolve(ui, '../../../engine-modules/rtklib-ex/dist/testdata/ubx_20080526.ubx')
const REQUIRED = ['Secure context', 'WebAssembly', 'WebAssembly SIMD', 'Web workers', 'Browser storage (OPFS)', 'Offline use (service worker)']

// A zip of stored entries, as an archiver might make one.
function zipOf(entries) {
  const locals = []
  const centrals = []
  let offset = 0
  for (const [name, text] of entries) {
    const n = Buffer.from(name)
    const data = Buffer.from(text)
    const head = Buffer.alloc(30)
    head.writeUInt32LE(0x04034b50, 0)
    head.writeUInt16LE(20, 4)
    head.writeUInt32LE(crc32(data), 14)
    head.writeUInt32LE(data.length, 18)
    head.writeUInt32LE(data.length, 22)
    head.writeUInt16LE(n.length, 26)
    const central = Buffer.alloc(46)
    central.writeUInt32LE(0x02014b50, 0)
    central.writeUInt16LE(20, 4)
    central.writeUInt16LE(20, 6)
    head.copy(central, 16, 14, 26)
    central.writeUInt16LE(n.length, 28)
    central.writeUInt32LE(offset, 42)
    locals.push(head, n, data)
    centrals.push(central, n)
    offset += 30 + n.length + data.length
  }
  const directory = Buffer.concat(centrals)
  const end = Buffer.alloc(22)
  end.writeUInt32LE(0x06054b50, 0)
  end.writeUInt16LE(entries.length, 8)
  end.writeUInt16LE(entries.length, 10)
  end.writeUInt32LE(directory.length, 12)
  end.writeUInt32LE(offset, 16)
  return Buffer.concat([...locals, directory, end])
}

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
  await page.getByLabel('Add files').setInputFiles(file)
  await page.getByRole('cell', { name: 'sample.txt' }).waitFor()
  await page.getByRole('button', { name: 'Compute SHA-256 hashes' }).click()
  await page.getByRole('cell', { name: expected }).waitFor()
  console.log(`browser storage project ok; core hash matches Node (${expected.slice(0, 12)}…)`)

  const zip = path.join(scratch, 'Survey.zip')
  writeFileSync(
    zip,
    zipOf([
      ['Survey/project.toml', 'schema = 1\nname = "Zip test"\n\n[adjustment]\nframe = "GDA2020"\n'],
      ['Survey/marks.toml', 'schema = 1\n'],
      ['Survey/occupations/bad.toml', 'not toml ['],
      ['Survey/.cache/ab/abcd', 'derived'],
    ]),
  )
  const importZip = page.getByLabel('Import a project zip')
  await importZip.setInputFiles(zip)
  await page.getByText('Survey (browser storage)').waitFor()
  await page.getByRole('cell', { name: 'occupations/bad.toml' }).waitFor()
  await page.getByRole('button', { name: 'Check project' }).click()
  await page.getByRole('status').filter({ hasText: 'Zip test: 0 marks, 0 occupations, 0 overlays; 1 problem.' }).waitFor()
  await page.locator('.checked li').filter({ hasText: 'occupations/bad.toml' }).waitFor()
  const exported = await downloaded('Export as zip')
  const text = exported.bytes.toString('latin1')
  if (exported.name !== 'Survey.zip' || !text.startsWith('PK\x03\x04') || !text.includes('occupations/bad.toml') || text.includes('.cache')) {
    throw new Error(`${exported.name} isn't the project without its cache`)
  }
  await importZip.setInputFiles(zip)
  await page.getByRole('alert').filter({ hasText: 'a project called Survey is already in browser storage' }).waitFor()
  page.once('dialog', (dialog) => dialog.accept())
  await page.getByRole('button', { name: 'Remove from browser storage' }).click()
  await page.getByRole('button', { name: 'Survey', exact: true }).waitFor({ state: 'detached' })
  console.log(`project zip imported, checked (1 problem, by file), exported without its cache (${exported.bytes.length} bytes), not imported twice, removed`)

  // 4. A RINEX file checked in slices, with navigation for coverage.
  await page.getByTestId('rinex-obs').setInputFiles(path.join(RINEX, 'rinex/SWTC00AUS_20251660000.rnx.crx'))
  await page.getByTestId('rinex-nav').setInputFiles(path.join(RINEX, 'nav/HOB200AUS_R_20251660000_excerpt_MN.rnx'))
  await page.getByRole('button', { name: 'Check', exact: true }).click()
  const checked = await page.getByTestId('rinex-result').innerText()
  const table = await page.getByTestId('rinex-result').locator('table').innerText()
  if (!checked.includes('SWTC') || !/Coverage above 10°/.test(checked)) throw new Error(`unexpected RINEX check:\n${checked}`)
  const timing = await page.getByTestId('rinex-timing').innerText()
  console.log(`checked a Hatanaka-compressed RINEX file with navigation: ${timing}`)

  // The same file gzipped, as archives publish it: the core gunzips it.
  const gz = path.join(scratch, 'SWTC00AUS_20251660000.crx.gz')
  writeFileSync(gz, gzipSync(readFileSync(path.join(RINEX, 'rinex/SWTC00AUS_20251660000.rnx.crx'))))
  await page.getByTestId('rinex-obs').setInputFiles(gz)
  await page.getByRole('button', { name: 'Check', exact: true }).click()
  await page.getByTestId('rinex-result').filter({ hasText: 'SWTC' }).waitFor()
  if (await page.getByTestId('rinex-converted').count()) throw new Error('a gzipped RINEX file was taken for raw data')
  if ((await page.getByTestId('rinex-result').locator('table').innerText()) !== table) throw new Error('the gzipped file checked differently')
  console.log('checked the same file gzipped: identical')

  // Raw u-blox data: converted with convbin, then checked with its own navigation.
  await page.getByTestId('rinex-nav').setInputFiles([])
  await page.getByTestId('rinex-obs').setInputFiles(UBX)
  if ((await page.getByTestId('rinex-raw-format').inputValue()) !== 'ubx') throw new Error('the UBX file wasn\'t recognised')
  await page.getByRole('button', { name: 'Check', exact: true }).click()
  await page.getByTestId('rinex-converted').filter({ hasText: 'ubx_20080526.obs' }).waitFor()
  const rawChecked = await page.getByTestId('rinex-result').innerText()
  if (!/242 of 242 epochs/.test(rawChecked) || !/GPS\s+9\b/.test(rawChecked)) throw new Error(`unexpected check of the converted file:\n${rawChecked}`)
  console.log('converted raw u-blox data with convbin and checked it: 242 epochs, 9 GPS satellites')

  // 5. Fetching CORS data for a project, from the browser. GA's hosts are
  //    answered here, so the test doesn't depend on them; CODE's isn't, and
  //    mustn't be asked without a relay. The observations are SWTC's, so
  //    their header can be checked against a site log, which here differs.
  const API = 'https://data.gnss.ga.gov.au/api/v1/rinexFiles'
  const S3 = 'https://geodesy-archive-permanent-storage-prod.s3.ap-southeast-2.amazonaws.com/default'
  const asked = []
  const obs = gzipSync(readFileSync(path.join(ui, '../../../crates/rinex-io/tests/data/rinex/SWTC00AUS_20251660000.rnx.crx')))
  const listing = (name, kind, size) =>
    JSON.stringify([
      { siteId: 'sydn', fileType: kind, filePeriod: '01D', startDate: '2025-06-15T00:00:00Z', rinexVersion: '3', fileLocation: `${S3}/${name}?sig=1`, fileSize: size },
    ])
  const siteLog = JSON.stringify({
    gnssReceivers: [{ type: 'LEICA GR30', serialNumber: '1706697', dateInstalled: '2020-01-01T00:00:00Z', dateRemoved: null }],
    gnssAntennas: [{ type: 'LEIAS10         NONE', serialNumber: '18171017', markerArpUpEcc: 1.9, dateInstalled: '2020-01-01T00:00:00Z', dateRemoved: null }],
  })
  const cors = { 'Access-Control-Allow-Origin': '*' }
  await context.route(/^https:\/\/(data\.gnss\.ga\.gov\.au|gws\.geodesy\.ga\.gov\.au|geodesy-archive-permanent-storage-prod|download\.aiub|files\.igs)/, (route) => {
    const url = route.request().url()
    asked.push(url)
    if (url.startsWith(API) && url.includes('fileType=obs') && url.includes('filePeriod=01D')) {
      return route.fulfill({ status: 200, headers: cors, contentType: 'application/json', body: listing('SYDN00AUS_R_20251660000_01D_30S_MO.crx.gz', 'obs', obs.length) })
    }
    if (url.startsWith(API) && url.includes('fileType=nav')) {
      return route.fulfill({ status: 200, headers: cors, contentType: 'application/json', body: listing('SYDN00AUS_R_20251660000_01D_MN.rnx.gz', 'nav', 4) })
    }
    if (url.includes('findByFourCharacterId?id=SYDN')) return route.fulfill({ status: 200, headers: cors, contentType: 'application/json', body: siteLog })
    if (url.startsWith(S3) && url.includes('_MO.')) return route.fulfill({ status: 200, headers: cors, body: obs })
    if (url.startsWith(S3)) return route.fulfill({ status: 200, headers: cors, body: 'data' })
    return route.fulfill({ status: 404, headers: cors, body: '' })
  })
  const survey = path.join(scratch, 'Fetch.zip')
  writeFileSync(
    survey,
    zipOf([
      ['Fetch/project.toml', 'schema = 1\nname = "Fetch test"\n\n[adjustment]\nframe = "GDA2020"\n'],
      ['Fetch/marks.toml', 'schema = 1\n\n[[mark]]\nid = "PM1"\napprox = [-33.815, 151.003, 20.0]\n'],
      [
        'Fetch/occupations/2025-06-15-PM1-A.toml',
        'schema = 1\nid = "2025-06-15-PM1-A"\nmark = "PM1"\nstart = "2025-06-15T03:10:00Z"\nend = "2025-06-15T05:10:00Z"\n\n[receiver]\nmake = "Trimble"\nmodel = "R12"\n\n[antenna]\nigs = "TRM159800.00"\nradome = "NONE"\n\n[height]\nmethod = "vertical"\nto = "ARP"\nvalue = 1.5\n',
      ],
    ]),
  )
  await page.getByLabel('Import a project zip').setInputFiles(survey)
  await page.getByText('Fetch (browser storage)').waitFor()
  await page.getByTestId('fetch-cors').fill('SYDN')
  await page.getByTestId('fetch-data').getByRole('button', { name: 'Fetch', exact: true }).click()
  const fetched = await page.getByTestId('fetch-result').innerText()
  // ANTEX comes from the app's own copy (data/), not the IGS or a relay.
  if (!/3 files \(3 downloaded\)/.test(fetched) || !fetched.includes('SYDN00AUS_R_20251660000_01D_30S_MO.crx.gz') || !/igs20_2434\.atx\.gz/.test(fetched) || !fetched.includes("the web app's copy")) {
    throw new Error(`unexpected fetch:\n${fetched}`)
  }
  const logs = await page.getByTestId('site-logs').innerText()
  if (!logs.includes('antenna height and offsets (H/E/N): the header says 1.9310/0.0000/0.0000 m, the site log 1.9000/0.0000/0.0000 m')) {
    throw new Error(`unexpected site-log check:\n${logs}`)
  }
  if (!fetched.includes("CORS site logs from Geoscience Australia's GNSS site log service")) throw new Error(`no site-log attribution:\n${fetched}`)
  await page.getByRole('cell', { name: 'data.toml' }).waitFor()
  if (asked.some((u) => u.includes('aiub') || u.includes('igs'))) throw new Error(`asked a relay-only host: ${asked.join(', ')}`)
  // Again: everything from the download cache in browser storage.
  const before = asked.filter((u) => u.startsWith(S3)).length
  await page.getByTestId('fetch-data').getByRole('button', { name: 'Fetch', exact: true }).click()
  await page.getByTestId('fetch-result').filter({ hasText: '3 files (0 downloaded)' }).waitFor()
  if (asked.filter((u) => u.startsWith(S3)).length !== before) throw new Error('fetched again what was cached')
  console.log('fetched SYDN observations and navigation data from (routed) GA hosts, and ANTEX from the app itself, into browser storage, then from the cache; checked the header against the site log')
  await context.unroute(/^https:/)

  // 6. Offline: wait for the service worker to finish caching, then reload
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
