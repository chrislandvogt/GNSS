// Client for the core worker. Domain logic lives in the core, never here
// (ARCHITECTURE.md section 8).
import type { CoreInfo, CoreRequest, CoreResponse } from '../workers/protocol'

type Pending = { resolve: (value: unknown) => void; reject: (reason: Error) => void }
// Omit that applies to each member of a union separately.
type WithoutId<T> = T extends unknown ? Omit<T, 'id'> : never

class CoreClient {
  private worker = new Worker(new URL('../workers/core.worker.ts', import.meta.url), { type: 'module' })
  private nextId = 1
  private pending = new Map<number, Pending>()

  constructor() {
    this.worker.onmessage = ({ data }: MessageEvent<CoreResponse>) => {
      const waiter = this.pending.get(data.id)
      if (!waiter) return
      this.pending.delete(data.id)
      if (data.ok) waiter.resolve(data.result)
      else waiter.reject(new Error(data.error))
    }
  }

  private call<T>(request: WithoutId<CoreRequest>, transfer: Transferable[] = []): Promise<T> {
    const id = this.nextId++
    return new Promise<T>((resolve, reject) => {
      this.pending.set(id, { resolve: resolve as (value: unknown) => void, reject })
      this.worker.postMessage({ ...request, id }, transfer)
    })
  }

  info(): Promise<CoreInfo> {
    return this.call<CoreInfo>({ type: 'info' })
  }

  /** SHA-256 of a file's contents, as recorded in raw.lock. */
  hash(data: ArrayBuffer): Promise<string> {
    return this.call<string>({ type: 'hash', data }, [data])
  }
}

export const core = new CoreClient()
