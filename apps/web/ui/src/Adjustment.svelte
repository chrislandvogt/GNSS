<script lang="ts">
  // The second-opinion adjustment (M3): load vectors and control, adjust,
  // read the report and download the results. Everything runs in the core
  // worker; files never leave the browser.
  import { onMount } from 'svelte'
  import { core } from './lib/core'
  import type { Adjusted, Catalogue, LoadedFile, ProjectSpec } from './workers/protocol'

  interface Entry extends LoadedFile {
    format: string
    frame: string
    epoch: string
    base: string
    rover: string
  }

  let catalogue = $state<Catalogue | null>(null)
  let entries = $state<Entry[]>([])
  let name = $state('Second opinion')
  let frame = $state('GDA2020')
  let epoch = $state('')
  let datum = $state<ProjectSpec['adjustment']['datum']>('constrained')
  let uncertainty = $state<ProjectSpec['adjustment']['uncertainty']>('a-priori')
  let edits = $state<ProjectSpec['edit']>([])
  let adjusted = $state<Adjusted | null>(null)
  let busy = $state(false)
  let error = $state('')
  let excluding = $state<string | null>(null)
  let reason = $state('')
  let dragging = $state(false)

  const dynamic = $derived(catalogue?.frames.some((f) => f.name === frame && !f.static) ?? false)
  const statesFrame = (format: string) => catalogue?.formats.find((f) => f.name === format)?.states_frame ?? false
  const ready = $derived(entries.length > 0 && entries.every((e) => e.format) && (!dynamic || epoch.trim() !== ''))

  onMount(async () => {
    try {
      catalogue = await core.catalogue()
    } catch (err) {
      error = `The core didn’t start: ${(err as Error).message}`
    }
  })

  async function addFiles(files: FileList | null | undefined) {
    if (!files) return
    error = ''
    for (const file of Array.from(files)) {
      try {
        const loaded = await core.addFile(file.name, await file.arrayBuffer())
        const entry: Entry = { ...loaded, format: loaded.format ?? '', frame: '', epoch: '', base: '', rover: '' }
        entries = [...entries.filter((e) => e.name !== loaded.name), entry]
      } catch (err) {
        error = `${file.name}: ${(err as Error).message}`
      }
    }
    adjusted = null
  }

  async function removeFile(fileName: string) {
    await core.removeFile(fileName)
    entries = entries.filter((e) => e.name !== fileName)
    edits = edits.filter((e) => !e.observation.startsWith(`${fileName}:`))
    adjusted = null
  }

  function spec(): ProjectSpec {
    const optional = (key: string, value: string) => (value.trim() ? { [key]: value.trim() } : {})
    return {
      schema: 1,
      name: name.trim() || 'Second opinion',
      adjustment: { frame, ...(dynamic ? { epoch: Number(epoch) } : {}), datum, uncertainty },
      import: entries.map((e) => ({
        file: `raw/${e.name}`,
        format: e.format,
        ...optional('frame', e.frame),
        ...optional('epoch', e.epoch),
        ...(e.format === 'rtklib' ? { ...optional('base', e.base), ...optional('rover', e.rover) } : {}),
      })),
      // Plain objects: reactive state can't be posted to the worker.
      edit: $state.snapshot(edits),
    }
  }

  async function run() {
    busy = true
    error = ''
    try {
      const now = new Date().toISOString()
      adjusted = await core.adjust(spec(), `${now.slice(0, 10)} ${now.slice(11, 19)} UTC`)
    } catch (err) {
      adjusted = null
      error = (err as Error).message
    } finally {
      busy = false
    }
  }

  async function exclude(observation: string) {
    edits = [...edits, { observation, action: 'exclude', reason: reason.trim() }]
    excluding = null
    reason = ''
    await run()
  }

  async function include(observation: string) {
    edits = edits.filter((e) => e.observation !== observation)
    await run()
  }

  function download(fileName: string, data: BlobPart, type = 'application/octet-stream') {
    const url = URL.createObjectURL(new Blob([data], { type }))
    const link = document.createElement('a')
    link.href = url
    link.download = fileName
    link.click()
    setTimeout(() => URL.revokeObjectURL(url), 60_000)
  }

  async function exportAs(format: string) {
    error = ''
    try {
      for (const file of await core.exportResults(format, 1000, new Date().toISOString())) download(file.name, file.data)
    } catch (err) {
      error = (err as Error).message
    }
  }

  async function downloadProject() {
    download('project.toml', await core.projectToml(spec()), 'text/plain')
  }

  const fmt = (v: number | null | undefined, digits = 2) => (v == null ? '–' : v.toFixed(digits))
  const size = (bytes: number) => (bytes >= 2 ** 20 ? `${(bytes / 2 ** 20).toFixed(1)} MiB` : `${(bytes / 1024).toFixed(1)} KiB`)
  const plural = (n: number, word: string) => `${n} ${word}${n === 1 ? '' : 's'}`
</script>

<section aria-labelledby="adjust-heading">
  <h2 id="adjust-heading">Second-opinion adjustment</h2>
  <p class="muted">
    Load GNSS vectors and control exported from any processing package, adjust them, and read the report. Everything
    runs in this browser; your files aren’t uploaded anywhere.
  </p>

  <h3>1. Files</h3>
  <div
    class="drop"
    class:dragging
    role="region"
    aria-label="Drop files here"
    ondragover={(e) => {
      e.preventDefault()
      dragging = true
    }}
    ondragleave={() => (dragging = false)}
    ondrop={(e) => {
      e.preventDefault()
      dragging = false
      addFiles(e.dataTransfer?.files)
    }}
  >
    <label
      >Add vectors and control: <input
        type="file"
        multiple
        data-testid="adjust-files"
        onchange={(e) => {
          const input = e.currentTarget as HTMLInputElement
          addFiles(input.files).then(() => (input.value = ''))
        }}
      /></label
    >
    <span class="muted">or drop them here. DynaML, STAR*NET, TDEF from TBC, RTKLIB .pos, SINEX or CSV.</span>
  </div>

  {#if entries.length}
    <table class="files">
      <thead>
        <tr><th>File</th><th class="num">Size</th><th>Format</th><th>Frame</th><th>Epoch</th><th></th></tr>
      </thead>
      <tbody>
        {#each entries as entry (entry.name)}
          <tr>
            <td>{entry.name}</td>
            <td class="num">{size(entry.bytes)}</td>
            <td>
              <select bind:value={entry.format} aria-label={`Format of ${entry.name}`}>
                <option value="">Choose…</option>
                {#each catalogue?.formats ?? [] as f (f.name)}<option value={f.name}>{f.label}</option>{/each}
              </select>
              {#if entry.format === 'rtklib'}
                <div class="inline">
                  <input placeholder="Base mark" bind:value={entry.base} aria-label={`Base mark of ${entry.name}`} />
                  <input placeholder="Rover mark" bind:value={entry.rover} aria-label={`Rover mark of ${entry.name}`} />
                </div>
              {/if}
            </td>
            <td>
              {#if statesFrame(entry.format)}
                <span class="muted">as the file says</span>
              {:else}
                <select bind:value={entry.frame} aria-label={`Frame of ${entry.name}`}>
                  <option value="">as the file says</option>
                  {#each catalogue?.frames ?? [] as f (f.name)}<option value={f.name}>{f.name}</option>{/each}
                </select>
              {/if}
            </td>
            <td>
              {#if !statesFrame(entry.format)}
                <input class="epoch" placeholder="e.g. 2026.5" bind:value={entry.epoch} aria-label={`Epoch of ${entry.name}`} />
              {/if}
            </td>
            <td><button class="link" onclick={() => removeFile(entry.name)}>Remove</button></td>
          </tr>
        {/each}
      </tbody>
    </table>
  {/if}

  <h3>2. Settings</h3>
  <div class="grid">
    <label>Name <input bind:value={name} /></label>
    <label
      >Adjust in
      <select bind:value={frame}>
        {#each catalogue?.frames ?? [] as f (f.name)}<option value={f.name}>{f.name}</option>{/each}
      </select>
    </label>
    {#if dynamic}
      <label>Epoch (decimal year) <input bind:value={epoch} placeholder="e.g. 2026.5" /></label>
    {/if}
    <label
      >Datum
      <select bind:value={datum}>
        <option value="constrained">Constrained by the control</option>
        <option value="minimal">Minimally constrained</option>
      </select>
    </label>
    <label
      >Uncertainties
      <select bind:value={uncertainty}>
        <option value="a-priori">A priori variance factor</option>
        <option value="a-posteriori">A posteriori variance factor</option>
      </select>
    </label>
  </div>
  <p class="actions">
    <button class="primary" onclick={run} disabled={busy || !ready}>{busy ? 'Adjusting…' : 'Adjust'}</button>
    <button onclick={downloadProject} disabled={!ready}>Download project.toml</button>
  </p>
  {#if error}<p class="error" role="alert">{error}</p>{/if}

  {#if adjusted}
    {@const s = adjusted.summary}
    <h3>3. Results</h3>
    <p class="verdict" class:pass={s.passed} class:fail={!s.passed} role="status">
      {#if !s.global_test}
        The network has no redundancy, so nothing could be tested.
      {:else if s.passed}
        The global test passed and no observation is suspect.
      {:else}
        The global test {s.global_test.passed ? 'passed' : 'failed'}; {plural(s.suspects.length, 'test')} of observations
        failed.
      {/if}
    </p>
    <dl class="facts">
      <dt>Marks</dt><dd>{s.marks}</dd>
      <dt>Observations</dt><dd>{s.observations}</dd>
      <dt>Degrees of freedom</dt><dd>{s.degrees_of_freedom}</dd>
      <dt>Variance factor</dt><dd>{fmt(s.variance_factor, 3)}</dd>
      {#if s.global_test}
        <dt>Global test</dt>
        <dd>{fmt(s.global_test.statistic)} {s.global_test.passed ? 'within' : 'outside'} [{fmt(s.global_test.lower)}, {fmt(s.global_test.upper)}]</dd>
      {/if}
      {#if s.loops}<dt>Loops</dt><dd>{s.loops[0]}, {s.loops[1]} failed</dd>{/if}
    </dl>

    {#if s.suspects.length}
      <h4>Suspects, worst first</h4>
      <p class="muted">Each is for you to judge: exclude it with a reason and adjust again, or keep it.</p>
      <table class="suspects">
        <thead>
          <tr><th>Observation</th><th>Marks</th><th>Test</th><th class="num">Statistic</th><th class="num">Critical</th><th></th></tr>
        </thead>
        <tbody>
          {#each s.suspects as suspect, i (i)}
            <tr>
              <td>{suspect.observation}{suspect.element > 1 ? `, element ${suspect.element}` : ''}</td>
              <td>{suspect.label}</td>
              <td>{suspect.test}</td>
              <td class="num">{fmt(suspect.statistic)}</td>
              <td class="num">{fmt(suspect.critical)}</td>
              <td>
                {#if excluding === `${i}`}
                  <form
                    class="inline"
                    onsubmit={(e) => {
                      e.preventDefault()
                      exclude(suspect.observation)
                    }}
                  >
                    <input bind:value={reason} placeholder="Why?" aria-label="Reason for excluding it" />
                    <button type="submit" disabled={!reason.trim() || busy}>Exclude</button>
                    <button type="button" class="link" onclick={() => (excluding = null)}>Cancel</button>
                  </form>
                {:else if !edits.some((e) => e.observation === suspect.observation)}
                  <button
                    class="link"
                    onclick={() => {
                      excluding = `${i}`
                      reason = ''
                    }}>Exclude…</button
                  >
                {/if}
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    {/if}

    {#if edits.length}
      <h4>Excluded</h4>
      <ul class="edits">
        {#each edits as edit (edit.observation)}
          <li>
            {edit.observation}: {edit.reason}
            <button class="link" onclick={() => include(edit.observation)} disabled={busy}>Include again</button>
          </li>
        {/each}
      </ul>
    {/if}

    {#if s.notes.length}
      <h4>Notes</h4>
      <ul class="notes">
        {#each s.notes as note, i (i)}<li>{note}</li>{/each}
      </ul>
    {/if}

    <h4>Downloads</h4>
    <p class="actions">
      <button onclick={() => download('report.html', adjusted!.report, 'text/html')}>Report (HTML)</button>
      {#each catalogue?.exports ?? [] as format (format.name)}
        <button onclick={() => exportAs(format.name)} title={format.description}>{format.label}</button>
      {/each}
    </p>

    <h4>Report</h4>
    <iframe class="report" title="Adjustment report" sandbox="" srcdoc={adjusted.report}></iframe>
  {/if}
</section>
