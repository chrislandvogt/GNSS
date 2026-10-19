// An engine worker: runs one engine module (ARCHITECTURE.md 9.2 and 14.1).
//
// The module is downloaded, checked against its pinned SHA-256 and compiled
// once. Every run gets a fresh instance with its own memory, which resets
// RTKLIB's file-level globals and contains any crash to that run.
//
// Inputs live in an in-memory file system. Reading inputs in slices from disk
// rather than copying them whole is planned for M8.
import {
  ConsoleStdout,
  File,
  OpenFile,
  PreopenDirectory,
  WASI,
  type Inode,
} from '@bjorn3/browser_wasi_shim'
import type { EngineInput, EngineRequest, EngineResponse, EngineRunRequest, EngineRunResult, WorkerScope } from './protocol'

const scope = self as unknown as WorkerScope<EngineRequest, EngineResponse>
const STDERR_TAIL = 4096
// Recently fetched inputs, so repeated runs on the same files don't refetch.
const MAX_CACHED_INPUTS = 8

let module: WebAssembly.Module | null = null
const inputCache = new Map<string, Uint8Array>()

async function sha256Hex(bytes: ArrayBuffer): Promise<string> {
  const digest = new Uint8Array(await crypto.subtle.digest('SHA-256', bytes))
  return Array.from(digest, (b) => b.toString(16).padStart(2, '0')).join('')
}

async function loadInput(input: EngineInput): Promise<Uint8Array> {
  if (input.data) return new Uint8Array(input.data)
  if (!input.url) throw new Error(`input ${input.name} has neither data nor a URL`)
  const cached = inputCache.get(input.url)
  if (cached) return cached
  const response = await fetch(input.url)
  if (!response.ok) throw new Error(`fetching ${input.url}: HTTP ${response.status}`)
  const bytes = new Uint8Array(await response.arrayBuffer())
  if (inputCache.size >= MAX_CACHED_INPUTS) inputCache.delete(inputCache.keys().next().value!)
  inputCache.set(input.url, bytes)
  return bytes
}

async function run(request: EngineRunRequest): Promise<EngineRunResult> {
  if (!module) throw new Error('engine module not initialised')
  const contents = new Map<string, Inode>()
  for (const input of request.inputs) {
    contents.set(input.name, new File(await loadInput(input), { readonly: true }))
  }
  const root = new PreopenDirectory('.', contents)

  const decoder = new TextDecoder()
  let stdout = ''
  let stderr = ''
  const fds = [
    new OpenFile(new File([])),
    new ConsoleStdout((bytes) => {
      stdout += decoder.decode(bytes, { stream: true })
    }),
    new ConsoleStdout((bytes) => {
      stderr = (stderr + decoder.decode(bytes, { stream: true })).slice(-STDERR_TAIL)
    }),
    root,
  ]
  const wasi = new WASI(['engine', ...request.args], [], fds)
  const instance = new WebAssembly.Instance(module, { wasi_snapshot_preview1: wasi.wasiImport })
  const exports = instance.exports as { memory: WebAssembly.Memory; _start: () => unknown }

  let exitCode: number
  let error: string | undefined
  const start = performance.now()
  try {
    exitCode = wasi.start({ exports })
  } catch (err) {
    // A trap (for example an out-of-bounds access in C code) fails this run only.
    exitCode = -1
    error = String(err)
  }
  const wallMs = performance.now() - start

  const outputs: Record<string, ArrayBuffer> = {}
  for (const name of request.outputs) {
    const node = root.dir.contents.get(name)
    if (node instanceof File) outputs[name] = node.data.slice().buffer
  }
  return {
    exitCode,
    wallMs,
    memoryBytes: exports.memory.buffer.byteLength,
    outputs,
    stdout,
    stderrTail: stderr,
    error,
  }
}

scope.onmessage = async ({ data: message }) => {
  if (message.type === 'init') {
    try {
      const response = await fetch(message.moduleUrl)
      const bytes = await response.arrayBuffer()
      const actual = await sha256Hex(bytes)
      if (actual !== message.sha256) {
        throw new Error(`engine module hash ${actual} doesn't match the pinned ${message.sha256}`)
      }
      module = await WebAssembly.compile(bytes)
      scope.postMessage({ type: 'ready' })
    } catch (err) {
      scope.postMessage({ type: 'init-failed', error: String(err) })
    }
    return
  }
  let result: EngineRunResult
  try {
    result = await run(message.request)
  } catch (err) {
    result = { exitCode: -1, wallMs: 0, memoryBytes: 0, outputs: {}, stdout: '', stderrTail: '', error: String(err) }
  }
  scope.postMessage({ type: 'result', id: message.id, result }, Object.values(result.outputs))
}
