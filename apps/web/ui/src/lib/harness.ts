// A hook for automated tests (the engine spike and end-to-end tests), enabled
// with ?harness in the URL. It exposes nothing a page script couldn't already
// do: it runs the same core and engine workers as the UI.
import { core } from './core'
import { rtklibPool } from './engine'
import type { EngineRunRequest } from '../workers/protocol'

export function installHarness(): void {
  const pool = rtklibPool()
  Object.assign(window, {
    __gnssHarness: {
      coreInfo: () => core.info(),
      engineReady: () => pool.ready(),
      async runEngine(request: EngineRunRequest) {
        const result = await pool.run(request)
        // Return outputs as text so the caller can hash them exactly.
        const decoder = new TextDecoder()
        const outputs = Object.fromEntries(
          Object.entries(result.outputs).map(([name, data]) => [name, decoder.decode(data)]),
        )
        return { ...result, outputs }
      },
    },
  })
}
