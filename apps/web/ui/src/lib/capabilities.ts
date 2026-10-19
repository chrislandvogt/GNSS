// Browser features the app needs (ARCHITECTURE.md 14.4), checked on start so
// anything missing can be explained rather than failing later.

export interface Capability {
  name: string
  ok: boolean
  /** What happens without it. */
  without: string
}

// A minimal module using a SIMD instruction; validates only where SIMD is supported.
const SIMD_PROBE = new Uint8Array([
  0, 97, 115, 109, 1, 0, 0, 0, 1, 5, 1, 96, 0, 1, 123, 3, 2, 1, 0, 10, 10, 1, 8, 0, 65, 0, 253, 15, 253, 98, 11,
])

function webgl(): boolean {
  try {
    const canvas = document.createElement('canvas')
    return Boolean(canvas.getContext('webgl2') ?? canvas.getContext('webgl'))
  } catch {
    return false
  }
}

export function detectCapabilities(): Capability[] {
  const wasm = typeof WebAssembly === 'object'
  return [
    { name: 'Secure context', ok: window.isSecureContext, without: 'storage and workers are restricted; use HTTPS or localhost' },
    { name: 'WebAssembly', ok: wasm, without: 'the core and engines can’t run' },
    { name: 'WebAssembly SIMD', ok: wasm && WebAssembly.validate(SIMD_PROBE), without: 'the core can’t run' },
    { name: 'Web workers', ok: typeof Worker === 'function', without: 'processing would freeze the page, so it’s disabled' },
    { name: 'Browser storage (OPFS)', ok: 'storage' in navigator && 'getDirectory' in navigator.storage, without: 'projects and the cache can’t be stored' },
    { name: 'Folders on disk', ok: 'showDirectoryPicker' in window, without: 'projects live in browser storage and move in and out as files' },
    { name: 'Offline use (service worker)', ok: 'serviceWorker' in navigator, without: 'the app needs a network connection' },
    { name: 'WebGL (map)', ok: webgl(), without: 'the map can’t be shown' },
  ]
}
