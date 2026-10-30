// The download cache in browser storage (OPFS), as the core worker sees it
// (ARCHITECTURE.md section 13). The same layout as natively, so packs and
// files move between them: objects/<first two hex digits>/<sha256>, found
// by index/<source>/<name>.json.
//
// The core can't wait on browser storage, so before a fetch it's given the
// index whole and the objects by name only (empty); only the CORS site list,
// which the fetch reads back, is given whole. What the fetch adds comes back
// as files to save.
import type { ProjectFileData } from './protocol'

/** The cache's folder at the root of the origin's browser storage. */
const CACHE = 'download-cache'

// Writable files in a worker: every engine has sync access handles.
interface SyncAccessHandle {
  truncate(size: number): void
  write(data: Uint8Array, options: { at: number }): number
  flush(): void
  close(): void
}
type SyncFileHandle = FileSystemFileHandle & { createSyncAccessHandle(): Promise<SyncAccessHandle> }

async function cacheDir(): Promise<FileSystemDirectoryHandle> {
  const root = await navigator.storage.getDirectory()
  return root.getDirectoryHandle(CACHE, { create: true })
}

async function subdirs(dir: FileSystemDirectoryHandle): Promise<[string, FileSystemDirectoryHandle][]> {
  const out: [string, FileSystemDirectoryHandle][] = []
  for await (const [name, entry] of dir.entries()) if (entry.kind === 'directory') out.push([name, entry as FileSystemDirectoryHandle])
  return out
}

async function files(dir: FileSystemDirectoryHandle): Promise<[string, FileSystemFileHandle][]> {
  const out: [string, FileSystemFileHandle][] = []
  for await (const [name, entry] of dir.entries()) if (entry.kind === 'file') out.push([name, entry as FileSystemFileHandle])
  return out
}

/** The cache, as the core is given it before a fetch. */
export async function loadCache(): Promise<ProjectFileData[]> {
  const dir = await cacheDir()
  const out: ProjectFileData[] = []
  // Objects the fetch reads back: the CORS site lists.
  const whole = new Set<string>()
  const index = await dir.getDirectoryHandle('index', { create: true })
  for (const [source, sourceDir] of await subdirs(index)) {
    for (const [name, handle] of await files(sourceDir)) {
      const data = await (await handle.getFile()).arrayBuffer()
      out.push({ path: `index/${source}/${name}`, data })
      if (source === 'ga-sites') {
        try {
          const sha = (JSON.parse(new TextDecoder().decode(data)) as { sha256: string }).sha256
          whole.add(`objects/${sha.slice(0, 2)}/${sha}`)
        } catch {
          // A damaged entry is a cache miss.
        }
      }
    }
  }
  const objects = await dir.getDirectoryHandle('objects', { create: true })
  for (const [prefix, prefixDir] of await subdirs(objects)) {
    for (const [name, handle] of await files(prefixDir)) {
      const path = `objects/${prefix}/${name}`
      out.push({ path, data: whole.has(path) ? await (await handle.getFile()).arrayBuffer() : new ArrayBuffer(0) })
    }
  }
  return out
}

/** Saves what a fetch added to the cache. */
export async function saveCache(written: { path: string; data: Uint8Array }[]): Promise<void> {
  const root = await cacheDir()
  for (const file of written) {
    const parts = file.path.split('/')
    let dir = root
    for (const part of parts.slice(0, -1)) dir = await dir.getDirectoryHandle(part, { create: true })
    const handle = (await dir.getFileHandle(parts[parts.length - 1], { create: true })) as SyncFileHandle
    const access = await handle.createSyncAccessHandle()
    try {
      access.truncate(0)
      access.write(file.data, { at: 0 })
      access.flush()
    } finally {
      access.close()
    }
  }
}

/**
 * A GET from the worker, synchronously, as the core asks for it: the body,
 * `null` for a missing file (404), or an error.
 */
export function httpGet(url: string): Uint8Array | null {
  const xhr = new XMLHttpRequest()
  xhr.open('GET', url, false)
  xhr.responseType = 'arraybuffer'
  try {
    xhr.send()
  } catch {
    throw new Error("couldn't reach it (offline, or the host doesn't allow the web app to download from it)")
  }
  if (xhr.status === 404) return null
  if (xhr.status < 200 || xhr.status >= 300) throw new Error(`HTTP ${xhr.status}`)
  return new Uint8Array(xhr.response as ArrayBuffer)
}
