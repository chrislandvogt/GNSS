// A pool of engine workers (ARCHITECTURE.md 14.1). Baselines run in parallel
// across workers; each worker runs one job at a time.
import { ENGINES } from '../generated/engines'
import type { EngineResponse, EngineRunRequest, EngineRunResult } from '../workers/protocol'

export type EngineId = keyof typeof ENGINES

interface Slot {
  worker: Worker
  ready: Promise<void>
  busy: boolean
}

export class EnginePool {
  private slots: Slot[] = []
  private queue: Array<() => void> = []
  private nextId = 1
  private waiting = new Map<number, (result: EngineRunResult) => void>()

  constructor(engine: EngineId, size = Math.max(1, (navigator.hardwareConcurrency || 2) - 1)) {
    const { url, sha256 } = ENGINES[engine]
    const moduleUrl = new URL(url, document.baseURI).href
    for (let i = 0; i < size; i++) {
      const worker = new Worker(new URL('../workers/engine.worker.ts', import.meta.url), { type: 'module' })
      const ready = new Promise<void>((resolve, reject) => {
        worker.onmessage = ({ data }: MessageEvent<EngineResponse>) => {
          if (data.type === 'ready') resolve()
          else if (data.type === 'init-failed') reject(new Error(data.error))
          else this.waiting.get(data.id)?.(data.result)
        }
      })
      worker.postMessage({ type: 'init', moduleUrl, sha256 })
      this.slots.push({ worker, ready, busy: false })
    }
  }

  /** Resolves when every worker has downloaded, verified and compiled the module. */
  ready(): Promise<void> {
    return Promise.all(this.slots.map((s) => s.ready)).then(() => undefined)
  }

  async run(request: EngineRunRequest): Promise<EngineRunResult> {
    const slot = await this.acquire()
    try {
      await slot.ready
      const id = this.nextId++
      return await new Promise<EngineRunResult>((resolve) => {
        this.waiting.set(id, (result) => {
          this.waiting.delete(id)
          resolve(result)
        })
        slot.worker.postMessage({ type: 'run', id, request })
      })
    } finally {
      slot.busy = false
      this.queue.shift()?.()
    }
  }

  private async acquire(): Promise<Slot> {
    for (;;) {
      const free = this.slots.find((s) => !s.busy)
      if (free) {
        free.busy = true
        return free
      }
      await new Promise<void>((resolve) => this.queue.push(resolve))
    }
  }
}

let pool: EnginePool | null = null

/** The shared RTKLIB-EX pool, created on first use. */
export function rtklibPool(): EnginePool {
  pool ??= new EnginePool('rtklib-ex')
  return pool
}

let converters: EnginePool | null = null

/** The convbin pool, for raw receiver data, created on first use. One
 *  worker: conversions are one file at a time. */
export function convbinPool(): EnginePool {
  converters ??= new EnginePool('convbin', 1)
  return converters
}
