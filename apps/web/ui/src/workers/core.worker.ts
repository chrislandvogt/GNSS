// The core worker: runs the Rust core, compiled to WebAssembly. It holds the
// loaded files and the last adjustment, so the page only sends edits.
import init, {
  addFile,
  adjust,
  buildTarget,
  catalogue,
  checkProject,
  exportResults,
  hashFile,
  projectToml,
  removeFile,
  version,
  zipExport,
  zipImport,
} from '../generated/core/gnss_web_core.js'
import type { CoreRequest, CoreResponse, ExportedFile, ProjectFileData, WorkerScope } from './protocol'

const scope = self as unknown as WorkerScope<CoreRequest, CoreResponse>
const ready = init()

const asBytes = (files: ProjectFileData[]) => files.map((f) => ({ path: f.path, data: new Uint8Array(f.data) }))

scope.onmessage = async ({ data: request }) => {
  const reply = (result: unknown, transfer: Transferable[] = []) =>
    scope.postMessage({ id: request.id, ok: true, result }, transfer)
  try {
    await ready
    switch (request.type) {
      case 'info':
        reply({ version: version(), build: buildTarget() })
        break
      case 'hash':
        reply(hashFile(new Uint8Array(request.data)))
        break
      case 'catalogue':
        reply(JSON.parse(catalogue()))
        break
      case 'add-file':
        reply(JSON.parse(addFile(request.name, new Uint8Array(request.data))))
        break
      case 'remove-file':
        removeFile(request.name)
        reply(null)
        break
      case 'adjust':
        reply(JSON.parse(adjust(JSON.stringify(request.project), request.generated)))
        break
      case 'export': {
        const files = (exportResults(request.format, request.ellipseScale, request.now) as { name: string; data: Uint8Array }[]).map(
          (f): ExportedFile => ({ name: f.name, data: f.data.buffer as ArrayBuffer }),
        )
        reply(
          files,
          files.map((f) => f.data),
        )
        break
      }
      case 'project-toml':
        reply(projectToml(JSON.stringify(request.project)))
        break
      case 'zip-export': {
        const zip = zipExport(asBytes(request.files))
        reply(zip.buffer, [zip.buffer])
        break
      }
      case 'zip-import': {
        const files = (zipImport(new Uint8Array(request.data)) as { path: string; data: Uint8Array }[]).map(
          (f): ProjectFileData => ({ path: f.path, data: f.data.buffer as ArrayBuffer }),
        )
        reply(
          files,
          files.map((f) => f.data),
        )
        break
      }
      case 'check-project':
        reply(JSON.parse(checkProject(asBytes(request.files))))
        break
    }
  } catch (err) {
    scope.postMessage({ id: request.id, ok: false, error: err instanceof Error ? err.message : String(err) })
  }
}
