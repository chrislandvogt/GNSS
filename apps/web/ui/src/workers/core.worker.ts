// The core worker: runs the Rust core, compiled to WebAssembly.
import init, { buildTarget, hashFile, version } from '../generated/core/gnss_web_core.js'
import type { CoreRequest, CoreResponse, WorkerScope } from './protocol'

const scope = self as unknown as WorkerScope<CoreRequest, CoreResponse>
const ready = init()

scope.onmessage = async ({ data: request }) => {
  try {
    await ready
    switch (request.type) {
      case 'info':
        scope.postMessage({ id: request.id, ok: true, result: { version: version(), build: buildTarget() } })
        break
      case 'hash':
        scope.postMessage({ id: request.id, ok: true, result: hashFile(new Uint8Array(request.data)) })
        break
    }
  } catch (err) {
    scope.postMessage({ id: request.id, ok: false, error: String(err) })
  }
}
