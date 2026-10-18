// Messages between the main thread and the workers. The main thread only
// renders; the core and engines run in workers (ARCHITECTURE.md 14.1).

export interface CoreInfo {
  version: string
  build: string
}

export type CoreRequest =
  | { id: number; type: 'info' }
  | { id: number; type: 'hash'; data: ArrayBuffer }

export type CoreResponse =
  | { id: number; ok: true; result: CoreInfo | string }
  | { id: number; ok: false; error: string }

/** An input file for an engine run, either inline or fetched by URL. */
export interface EngineInput {
  name: string
  url?: string
  data?: ArrayBuffer
}

export interface EngineRunRequest {
  /** Arguments after the program name. */
  args: string[]
  inputs: EngineInput[]
  /** Files to collect after the run, relative to the working directory. */
  outputs: string[]
}

export interface EngineRunResult {
  exitCode: number
  /** Time spent inside the module, excluding fetching inputs. */
  wallMs: number
  /** Size of the module's linear memory at the end of the run (it only grows). */
  memoryBytes: number
  outputs: Record<string, ArrayBuffer>
  stdout: string
  stderrTail: string
  error?: string
}

export type EngineRequest =
  | { type: 'init'; moduleUrl: string; sha256: string }
  | { type: 'run'; id: number; request: EngineRunRequest }

export type EngineResponse =
  | { type: 'ready' }
  | { type: 'init-failed'; error: string }
  | { type: 'result'; id: number; result: EngineRunResult }

/** The subset of a dedicated worker's global scope these workers use. */
export interface WorkerScope<In, Out> {
  onmessage: ((event: MessageEvent<In>) => void) | null
  postMessage(message: Out, transfer?: Transferable[]): void
}
