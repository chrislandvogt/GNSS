<script lang="ts">
  // Checking a RINEX observation file (M6): equipment, completeness, gaps,
  // slips, multipath and, with navigation files, elevation coverage. The
  // core worker reads the file a slice at a time, so a day at 1 Hz fits.
  // Raw receiver data is converted to RINEX first, with convbin (D-020).
  import { core } from './lib/core'
  import { RAW_FORMATS, convertRaw, guessFormat, looksLikeRinex, needsDate, type Converted, type RawFormat } from './lib/convert'
  import { download } from './lib/download'
  import type { RinexChecked } from './workers/protocol'

  // Raw: Files go to the worker as they are, not as reactive proxies.
  let obs = $state.raw<File | null>(null)
  let nav = $state.raw<File[]>([])
  let result = $state<RinexChecked | null>(null)
  let busy = $state(false)
  let error = $state('')
  // Set when the observation file isn't RINEX.
  let raw = $state<RawFormat | null>(null)
  let rawDate = $state('')
  let converted = $state.raw<Converted | null>(null)

  const pct = (x: number | null | undefined) => (x == null ? '—' : `${(100 * x).toFixed(1)}%`)
  const mb = (bytes: number) => (bytes < 1e6 ? `${(bytes / 1e3).toFixed(0)} kB` : `${(bytes / 1e6).toFixed(1)} MB`)

  async function choose(file: File | null) {
    obs = file
    converted = null
    raw = null
    if (file && !(await looksLikeRinex(file))) {
      raw = guessFormat(file.name) ?? 'ubx'
      rawDate = new Date(file.lastModified).toISOString().slice(0, 10)
    }
  }

  async function check() {
    if (!obs) return
    busy = true
    error = ''
    result = null
    try {
      let file = obs
      let navs = nav
      if (raw) {
        converted = await convertRaw(obs, raw, needsDate(raw) ? rawDate : undefined)
        file = converted.obs
        if (converted.nav) navs = [converted.nav, ...nav]
      }
      result = await core.checkRinex(file, navs)
    } catch (err) {
      error = `${obs.name}: ${(err as Error).message}`
    } finally {
      busy = false
    }
  }
</script>

<section>
  <h2>Check a RINEX file</h2>
  <p class="muted">
    RINEX 2, 3 or 4, plain or Hatanaka-compressed (not gzipped), or raw data from an open receiver format (u-blox,
    Septentrio, NovAtel, Unicore, RTCM and others), converted to RINEX 3.04 with RTKLIB-EX's convbin. Navigation files
    add elevation coverage: how much of what was above 10° was tracked.
  </p>
  <p class="actions">
    <label>Observations:
      <input type="file" data-testid="rinex-obs" onchange={(e) => choose(e.currentTarget.files?.[0] ?? null)} />
    </label>
    {#if raw}
      <label>Raw format:
        <select bind:value={raw} data-testid="rinex-raw-format">
          {#each Object.entries(RAW_FORMATS) as [id, label] (id)}<option value={id}>{label}</option>{/each}
        </select>
      </label>
      {#if needsDate(raw)}
        <label>Logged on (UTC): <input type="date" bind:value={rawDate} required /></label>
      {/if}
    {/if}
    <label>Navigation (optional):
      <input type="file" multiple data-testid="rinex-nav" onchange={(e) => (nav = Array.from(e.currentTarget.files ?? []))} />
    </label>
    <button onclick={check} disabled={busy || !obs}>{busy ? 'Checking…' : 'Check'}</button>
  </p>
  {#if error}<p class="error" role="alert">{error}</p>{/if}
  {#if result}
    {@const q = result.qc}
    <div role="status" data-testid="rinex-result">
      {#if converted && raw}
        <p data-testid="rinex-converted">
          Converted from {RAW_FORMATS[raw]} with convbin:
          <button class="link" onclick={() => download(converted!.obs.name, converted!.obs)}>{converted.obs.name}</button>
          {#if converted.nav}and <button class="link" onclick={() => download(converted!.nav!.name, converted!.nav!)}>{converted.nav.name}</button>{/if}
        </p>
      {/if}
      <p>
        <strong>{q.marker || '(no marker name)'}</strong> (RINEX {q.version}): {q.receiver} with {q.antenna}
        {q.radome ?? 'NONE'}{#if q.antenna_delta}, antenna height {q.antenna_delta[0].toFixed(4)} m{/if}.
      </p>
      {#if q.antenna_note}<p class="muted">Note: {q.antenna_note}</p>{/if}
      {#if q.coverage_note}<p class="muted">Note: {q.coverage_note}</p>{/if}
      <p>
        {q.first ?? '?'} to {q.last ?? '?'} {q.time_system}: {q.epochs} of {q.expected_epochs ?? '?'} epochs
        ({pct(q.expected_epochs ? q.epochs / q.expected_epochs : null)}) every {q.interval ?? '?'} s;
        {q.gaps.length} gaps; {q.events} events.
      </p>
      <table>
        <thead>
          <tr>
            <th>System</th><th>Satellites</th><th>Pair</th><th>Slips</th><th>Receiver-flagged</th><th>Outliers</th>
            <th>Multipath</th><th>Coverage above 10°</th>
          </tr>
        </thead>
        <tbody>
          {#each q.systems as s (s.system)}
            <tr>
              <td>{s.system}</td>
              <td class="num">{s.satellites}</td>
              <td>{s.pair ?? '—'}</td>
              <td class="num">{s.pair ? s.slips : '—'}</td>
              <td class="num">{s.pair ? s.flagged : '—'}</td>
              <td class="num">{s.pair ? s.outliers : '—'}</td>
              <td>{s.multipath.map(([band, rms]) => `${band} ${rms.toFixed(3)} m`).join(', ') || '—'}</td>
              <td class="num">{s.coverage ? pct(s.coverage.ratio) : '—'}</td>
            </tr>
          {/each}
        </tbody>
      </table>
      <p class="muted" data-testid="rinex-timing">
        Read {mb(result.bytes)} in {result.seconds.toFixed(2)} s ({mb(result.bytes / Math.max(result.seconds, 1e-3))}/s).
      </p>
    </div>
  {/if}
</section>
