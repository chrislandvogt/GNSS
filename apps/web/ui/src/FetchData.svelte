<script lang="ts">
  // Fetching what a project's occupations need (M7, ARCHITECTURE.md
  // section 13): CORS observations and navigation data straight from
  // Geoscience Australia, which the browser can reach; the ANTEX file from
  // the app's own copy (D-028); and, only through a relay the user turns
  // on, CODE's precise products.
  // Files go into the download cache in browser storage; data.toml records
  // which were used.
  import { core } from './lib/core'
  import { readProject, writeProjectFile, type ProjectRoot } from './lib/project'
  import type { FetchOutcome } from './workers/protocol'

  let { project, onsaved }: { project: ProjectRoot; onsaved: () => void } = $props()

  let cors = $state('')
  let nearest = $state(2)
  let withinKm = $state(100)
  let useRelay = $state(false)
  let relay = $state('http://127.0.0.1:8787')
  let precise = $state(false)
  let antex = $state(true)
  let busy = $state(false)
  let error = $state('')
  let outcome = $state<FetchOutcome | null>(null)
  // Served by `gnss serve`, whose relay is beside the app on this machine:
  // its pages carry a cookie saying so, confirmed by asking the relay.
  let localRelay = $state(false)

  $effect(() => {
    if (!document.cookie.split('; ').includes('gnss-serve=relay')) return
    const here = new URL('relay', document.baseURI).href
    fetch(here, { cache: 'no-store' })
      .then(async (r) => r.ok && (await r.text()).startsWith('gnss relay'))
      .then((found) => {
        if (found) {
          localRelay = true
          relay = here
        }
      })
      .catch(() => {})
  })

  const kb = (bytes: number) => (bytes < 1e6 ? `${(bytes / 1e3).toFixed(0)} kB` : `${(bytes / 1e6).toFixed(1)} MB`)

  async function fetchData() {
    busy = true
    error = ''
    outcome = null
    try {
      const result = await core.fetchData(await readProject(project), {
        cors: cors
          .split(/[\s,]+/)
          .map((s) => s.trim().toUpperCase())
          .filter(Boolean),
        nearest,
        within_km: withinKm,
        precise: useRelay && precise,
        antex,
        app_data: new URL('data/', document.baseURI).href,
        relay: useRelay ? relay : null,
        now: new Date().toISOString(),
      })
      await writeProjectFile(project, 'data.toml', result.data_toml)
      outcome = result
      onsaved()
    } catch (err) {
      error = (err as Error).message
    } finally {
      busy = false
    }
  }
</script>

<div class="fetch" data-testid="fetch-data">
  <h3>Fetch CORS data</h3>
  <p class="muted">
    Finds what the occupations need from outside the project: CORS observations over each occupation, and broadcast
    navigation data, from Geoscience Australia's GNSS Data Repository, and antenna calibrations from the IGS, which
    this app serves itself. Files are kept in this browser's download cache,
    shared by every project; <code>data.toml</code> records which were used, with their licences.
  </p>
  <p class="actions">
    <label>CORS: <input bind:value={cors} placeholder="nearest" size="16" data-testid="fetch-cors" /></label>
    {#if !cors.trim()}
      <label>the nearest <input type="number" min="1" max="6" bind:value={nearest} /></label>
      <label>within <input type="number" min="1" max="1000" bind:value={withinKm} /> km</label>
    {/if}
    <label><input type="checkbox" bind:checked={antex} data-testid="fetch-antex" /> ANTEX (igs20.atx, week 2434)</label>
  </p>
  <details>
    <summary>Precise products (needs a relay)</summary>
    <p class="muted">
      CODE's precise orbits and clocks come from a host that doesn't let a web page download from it. A relay (<code>gnss-relay</code>, which you or your organisation run) can fetch them on the app's
      behalf. <strong>A relay sees which files are requested</strong>: station names and dates, which hint at where and
      when a survey happened. It keeps no logs, but use one only where you'd trust it to see that. Or bring the files
      in a data pack (<code>gnss data export</code> on a connected machine).
    </p>
    {#if localRelay}
      <p class="muted" data-testid="fetch-local-relay">
        This app is served by <code>gnss serve</code>, whose relay runs on this machine, so only you see what it
        fetches; the sources still see the requests, as they would from the CLI.
      </p>
    {/if}
    <p class="actions">
      <label><input type="checkbox" bind:checked={useRelay} data-testid="fetch-use-relay" /> Use a relay:</label>
      <input bind:value={relay} size="28" disabled={!useRelay} />
    </p>
    <p class="actions">
      <label><input type="checkbox" bind:checked={precise} disabled={!useRelay} /> Precise orbits, clocks and ERPs</label>
    </p>
  </details>
  <p class="actions"><button onclick={fetchData} disabled={busy}>{busy ? 'Fetching…' : 'Fetch'}</button></p>
  {#if error}<p class="error" role="alert">{error}</p>{/if}
  {#if outcome}
    <div role="status" data-testid="fetch-result">
      <p>
        {outcome.fetched.length} files ({outcome.fetched.filter((f) => f.downloaded).length} downloaded), recorded in
        <code>data.toml</code>{outcome.problems.length ? `; ${outcome.problems.length} not fetched` : ''}.
      </p>
      <table>
        <thead><tr><th>For</th><th>File</th><th>Size</th><th></th></tr></thead>
        <tbody>
          {#each outcome.fetched as f (f.purpose + f.name)}
            <tr>
              <td>{f.purpose}{#if f.note}<br /><span class="muted">{f.note}</span>{/if}</td>
              <td>{f.name}</td>
              <td class="num">{kb(f.bytes)}</td>
              <td>{f.downloaded ? 'downloaded' : 'cached'}</td>
            </tr>
          {/each}
        </tbody>
      </table>
      {#if outcome.notes.length || outcome.problems.length}
        <ul>
          {#each outcome.notes as n, i (i)}<li class="muted">{n}</li>{/each}
          {#each outcome.problems as p, i (i)}<li>{p}</li>{/each}
        </ul>
      {/if}
      {#if outcome.site_logs.length}
        <h4>Site logs</h4>
        <ul data-testid="site-logs">
          {#each outcome.site_logs as c (c.file)}
            {#if c.findings.length}
              {#each c.findings as f, i (i)}<li>{c.file} ({c.station}): {f}</li>{/each}
            {:else}
              <li class="muted">{c.file}: the header agrees with {c.station}'s site log</li>
            {/if}
          {/each}
        </ul>
      {/if}
      {#if outcome.attributions.length}
        <p class="muted">{outcome.attributions.join(' ')}</p>
      {/if}
    </div>
  {/if}
</div>
