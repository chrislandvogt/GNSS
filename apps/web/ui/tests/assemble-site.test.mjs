// The released site's assembly (scripts/assemble-site.mjs), on made-up
// release bundles. Run with: node --test tests/assemble-site.test.mjs
import assert from 'node:assert/strict'
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync, existsSync } from 'node:fs'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { test } from 'node:test'
import { assemble, mergeHeaders, styleHashes } from '../scripts/assemble-site.mjs'

const headers = (hash) => `# Response headers
/*
  Content-Security-Policy: default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self' 'sha256-${hash}='; img-src 'self' data: blob:
  X-Content-Type-Options: nosniff

/sw.js
  Cache-Control: no-cache
`

function release(root, version, hash) {
  const dir = path.join(root, version)
  mkdirSync(path.join(dir, 'assets'), { recursive: true })
  writeFileSync(path.join(dir, 'index.html'), `<!doctype html><title>${version}</title>`)
  writeFileSync(path.join(dir, 'sw.js'), `// ${version}`)
  writeFileSync(path.join(dir, 'assets', 'app.js'), `console.log('${version}')`)
  writeFileSync(path.join(dir, '_headers'), headers(hash))
}

test('the latest release is at the root and every release under /v/', () => {
  const work = mkdtempSync(path.join(tmpdir(), 'assemble-'))
  try {
    const releases = path.join(work, 'releases')
    release(releases, '0.2.0', 'B')
    release(releases, '0.10.0', 'C')
    release(releases, '0.1.0', 'A')
    const out = path.join(work, 'site')
    const { versions, latest } = assemble(releases, out)
    assert.deepEqual(versions, ['0.1.0', '0.2.0', '0.10.0'])
    assert.equal(latest, '0.10.0')
    assert.match(readFileSync(path.join(out, 'index.html'), 'utf8'), /0\.10\.0/)
    for (const v of versions) {
      assert.match(readFileSync(path.join(out, 'v', v, 'index.html'), 'utf8'), new RegExp(v.replaceAll('.', '\\.')))
      assert.match(readFileSync(path.join(out, 'v', v, 'assets', 'app.js'), 'utf8'), new RegExp(v.replaceAll('.', '\\.')))
      assert.ok(!existsSync(path.join(out, 'v', v, '_headers')), `no _headers under /v/${v}/`)
    }
    // One policy for all, allowing every release's report style.
    const merged = readFileSync(path.join(out, '_headers'), 'utf8')
    assert.deepEqual(styleHashes(merged), ["'sha256-A='", "'sha256-B='", "'sha256-C='"])
    assert.match(merged, /style-src 'self' 'sha256-A=' 'sha256-B=' 'sha256-C='; img-src/)
    assert.match(merged, /\/v\/:version\/sw\.js\n {2}Cache-Control: no-cache\n$/)
    assert.throws(() => assemble(releases, out), /already exists/)
  } finally {
    rmSync(work, { recursive: true, force: true })
  }
})

test('refuses what it cannot assemble safely', () => {
  const work = mkdtempSync(path.join(tmpdir(), 'assemble-'))
  try {
    const releases = path.join(work, 'releases')
    mkdirSync(releases)
    assert.throws(() => assemble(releases, path.join(work, 'a')), /no releases/)
    release(releases, '0.1.0', 'A')
    mkdirSync(path.join(releases, 'latest'))
    assert.throws(() => assemble(releases, path.join(work, 'b')), /latest isn't a release version/)
    assert.throws(() => mergeHeaders('/*\n  X-Frame-Options: DENY\n', []), /no Content-Security-Policy/)
  } finally {
    rmSync(work, { recursive: true, force: true })
  }
})

test("the app's own headers have the report's style hash", () => {
  const own = readFileSync(new URL('../public/_headers', import.meta.url), 'utf8')
  assert.equal(styleHashes(own).length, 1)
  const merged = mergeHeaders(own, [own])
  assert.deepEqual(styleHashes(merged), styleHashes(own))
})
