// Raw receiver data to RINEX with RTKLIB-EX's convbin, run as a WebAssembly
// module in an engine worker (D-020). The RINEX it writes is then read like
// any other.
import { convbinPool } from './engine'

/** The formats convbin reads, by its `-r` name. */
export const RAW_FORMATS = {
  rtcm3: 'RTCM 3',
  rtcm2: 'RTCM 2',
  ubx: 'u-blox (UBX)',
  sbf: 'Septentrio (SBF)',
  nov: 'NovAtel (OEM4/6/7)',
  unicore: 'Unicore',
  javad: 'Javad (GREIS)',
  hemis: 'Hemisphere',
  binex: 'BINEX',
  rt17: 'Trimble RT17',
  sbp: 'Swift (SBP)',
  stq: 'SkyTraq',
  nvs: 'NVS (BINR)',
} as const
export type RawFormat = keyof typeof RAW_FORMATS

const BY_EXTENSION: Record<string, RawFormat> = {
  rtcm3: 'rtcm3',
  rtcm2: 'rtcm2',
  ubx: 'ubx',
  sbf: 'sbf',
  gps: 'nov',
  jps: 'javad',
  bin: 'hemis',
  bnx: 'binex',
  binex: 'binex',
  rt17: 'rt17',
  sbp: 'sbp',
  stq: 'stq',
  nvs: 'nvs',
}

/** The format a file's extension suggests, if any. */
export function guessFormat(name: string): RawFormat | null {
  const ext = name.split('.').pop()?.toLowerCase() ?? ''
  return BY_EXTENSION[ext] ?? null
}

/** Whether a format needs an approximate date: RTCM carries no GPS week. */
export function needsDate(format: RawFormat): boolean {
  return format === 'rtcm3' || format === 'rtcm2'
}

/** Whether a file starts like RINEX: `RINEX VERSION / TYPE` (or a CRINEX
 *  header) in the first record. Gzipped files count: the core reads them,
 *  and archives publish RINEX that way. */
export async function looksLikeRinex(file: File): Promise<boolean> {
  const magic = new Uint8Array(await file.slice(0, 2).arrayBuffer())
  if (magic[0] === 0x1f && magic[1] === 0x8b) return true
  const head = await file.slice(0, 160).text()
  const first = head.split('\n')[0] ?? ''
  return first.includes('RINEX VERSION / TYPE') || first.includes('CRINEX VERS')
}

export interface Converted {
  obs: File
  nav: File | null
  /** convbin's progress and messages, for when it finds nothing. */
  log: string
}

/**
 * Converts a raw receiver file to RINEX 3.04 observations (with signal
 * strengths) and navigation. `date` (YYYY-MM-DD) is required for RTCM.
 */
export async function convertRaw(file: File, format: RawFormat, date?: string): Promise<Converted> {
  if (needsDate(format) && !date) throw new Error('RTCM needs the date it was logged')
  const stem = file.name.replace(/\.[^.]*$/, '') || 'raw'
  const input = `input.${format}`
  const args = ['-r', format, '-os', '-o', 'out.obs', '-n', 'out.nav']
  if (date) args.push('-tr', date.replaceAll('-', '/'), '00:00:00')
  args.push(input)
  const result = await convbinPool().run({
    args,
    inputs: [{ name: input, data: await file.arrayBuffer() }],
    outputs: ['out.obs', 'out.nav'],
  })
  const log = result.error ?? result.stderrTail
  if (result.exitCode !== 0) throw new Error(`convbin exited with ${result.exitCode}: ${log.slice(-300)}`)
  const obs = result.outputs['out.obs']
  // An observation file with no epochs is just a header.
  const header = obs ? new TextDecoder().decode(obs.slice(0, 1 << 16)) : ''
  const end = header.indexOf('END OF HEADER')
  if (!obs || end < 0 || obs.byteLength <= end + 'END OF HEADER'.length + 2) {
    throw new Error(`no observations found in ${file.name} as ${RAW_FORMATS[format]}`)
  }
  const nav = result.outputs['out.nav']
  return {
    obs: new File([obs], `${stem}.obs`),
    nav: nav && nav.byteLength > 0 ? new File([nav], `${stem}.nav`) : null,
    log,
  }
}
