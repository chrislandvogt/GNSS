<script lang="ts">
  import { onMount } from 'svelte'
  import Adjustment from './Adjustment.svelte'
  import { detectCapabilities } from './lib/capabilities'
  import { core } from './lib/core'
  import { rtklibPool } from './lib/engine'
  import { ENGINE_CHECK_CONFIG } from './lib/engine-check'
  import {
    canOpenFolders,
    importFiles,
    listFiles,
    openBrowserStorage,
    openFolder,
    type ProjectFile,
    type ProjectRoot,
  } from './lib/project'
  import type { CoreInfo } from './workers/protocol'

  const capabilities = detectCapabilities()
  let info = $state<CoreInfo | null>(null)
  let project = $state<ProjectRoot | null>(null)
  let files = $state<ProjectFile[]>([])
  let hashes = $state<Record<string, string>>({})
  let message = $state('')
  let busy = $state(false)

  let rover = $state('')
  let base = $state('')
  let nav = $state('')
  let solution = $state('')

  onMount(async () => {
    try {
      info = await core.info()
    } catch (err) {
      message = `The core didn’t start: ${err}`
    }
  })

  async function open(kind: 'folder' | 'browser') {
    try {
      project = kind === 'folder' ? await openFolder() : await openBrowserStorage()
      await refresh()
    } catch (err) {
      if ((err as DOMException).name !== 'AbortError') message = String(err)
    }
  }

  async function refresh() {
    if (!project) return
    files = await listFiles(project.handle)
    hashes = {}
  }

  async function onImport(event: Event) {
    const input = event.currentTarget as HTMLInputElement
    if (project && input.files) {
      await importFiles(project, input.files)
      await refresh()
    }
  }

  async function hashAll() {
    busy = true
    message = ''
    try {
      for (const f of files) {
        const data = await (await f.handle.getFile()).arrayBuffer()
        hashes[f.path] = await core.hash(data)
      }
    } catch (err) {
      message = String(err)
    } finally {
      busy = false
    }
  }

  async function read(path: string): Promise<ArrayBuffer> {
    const file = files.find((f) => f.path === path)
    if (!file) throw new Error(`${path} isn’t in the project`)
    return (await file.handle.getFile()).arrayBuffer()
  }

  async function runEngineCheck() {
    busy = true
    message = ''
    solution = ''
    try {
      const result = await rtklibPool().run({
        args: ['-k', 'check.conf', '-o', 'out.pos', 'rover.obs', 'base.obs', 'nav.rnx'],
        inputs: [
          { name: 'check.conf', data: new TextEncoder().encode(ENGINE_CHECK_CONFIG).buffer as ArrayBuffer },
          { name: 'rover.obs', data: await read(rover) },
          { name: 'base.obs', data: await read(base) },
          { name: 'nav.rnx', data: await read(nav) },
        ],
        outputs: ['out.pos'],
      })
      const text = result.outputs['out.pos'] ? new TextDecoder().decode(result.outputs['out.pos']) : ''
      const lines = text.split('\n').filter((l) => l && !l.startsWith('%'))
      solution =
        `exit ${result.exitCode} in ${(result.wallMs / 1000).toFixed(1)} s, ` +
        `${(result.memoryBytes / 2 ** 20).toFixed(0)} MiB of module memory\n` +
        (lines.join('\n') || result.error || result.stderrTail)
    } catch (err) {
      message = String(err)
    } finally {
      busy = false
    }
  }

  const fmtSize = (bytes: number) =>
    bytes >= 2 ** 20 ? `${(bytes / 2 ** 20).toFixed(1)} MiB` : `${(bytes / 1024).toFixed(1)} KiB`
</script>

<main>
  <header>
    <h1>GNSS survey processing</h1>
    <p class="muted">
      Working title. A second opinion on GNSS networks —
      {#if info}core {info.version} ({info.build} build){:else}starting the core…{/if}
    </p>
  </header>

  <Adjustment />

  <details class="diagnostics">
  <summary>Diagnostics: browser features, project storage and the baseline engine</summary>

  {#if message}<p class="error" role="alert">{message}</p>{/if}

  <section>
    <h2>Browser check</h2>
    <ul class="checks">
      {#each capabilities as c (c.name)}
        <li class:missing={!c.ok}>
          <span aria-hidden="true">{c.ok ? '✓' : '✗'}</span>
          {c.name}{#if !c.ok}<span class="muted"> — without it, {c.without}</span>{/if}
        </li>
      {/each}
    </ul>
  </section>

  <section>
    <h2>Project</h2>
    <p class="actions">
      {#if canOpenFolders}
        <button onclick={() => open('folder')}>Open a project folder</button>
      {/if}
      <button onclick={() => open('browser')}>Use browser storage</button>
    </p>
    {#if project}
      <p>
        <strong>{project.name}</strong>
        <span class="muted">({project.kind === 'folder' ? 'folder on disk' : 'browser storage'}, {files.length} files)</span>
      </p>
      {#if project.kind === 'browser'}
        <label>Add files: <input type="file" multiple onchange={onImport} /></label>
      {/if}
      {#if files.length}
        <p class="actions">
          <button onclick={hashAll} disabled={busy}>Compute SHA-256 hashes</button>
        </p>
        <table>
          <thead><tr><th>File</th><th>Size</th><th>SHA-256</th></tr></thead>
          <tbody>
            {#each files as f (f.path)}
              <tr>
                <td>{f.path}</td>
                <td class="num">{fmtSize(f.size)}</td>
                <td class="hash">{hashes[f.path] ?? ''}</td>
              </tr>
            {/each}
          </tbody>
        </table>
      {/if}
    {/if}
  </section>

  {#if files.length}
    <section>
      <h2>Engine check</h2>
      <p class="muted">
        Runs RTKLIB-EX in an engine worker with a fixed static configuration. Choose two observation files and a
        navigation file.
      </p>
      <div class="grid">
        {#each [['Rover observations', 'rover'], ['Base observations', 'base'], ['Navigation', 'nav']] as [label, key] (key)}
          <label>
            {label}
            <select
              value={key === 'rover' ? rover : key === 'base' ? base : nav}
              onchange={(e) => {
                const v = (e.currentTarget as HTMLSelectElement).value
                if (key === 'rover') rover = v
                else if (key === 'base') base = v
                else nav = v
              }}
            >
              <option value="">Choose…</option>
              {#each files as f (f.path)}<option value={f.path}>{f.path}</option>{/each}
            </select>
          </label>
        {/each}
      </div>
      <p class="actions">
        <button onclick={runEngineCheck} disabled={busy || !rover || !base || !nav}>Run static solution</button>
      </p>
      {#if solution}<pre>{solution}</pre>{/if}
    </section>
  {/if}
  </details>
</main>
