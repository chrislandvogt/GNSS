// Client for the core worker. Domain logic lives in the core, never here
// (ARCHITECTURE.md section 8).
import type {
  Adjusted,
  Catalogue,
  CheckedProject,
  CoreInfo,
  CoreRequest,
  CoreResponse,
  ExportedFile,
  FetchOptions,
  FetchOutcome,
  LoadedFile,
  ProjectFileData,
  ProjectSpec,
  RinexChecked,
} from '../workers/protocol'

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

  /** The frames, import formats and export formats on offer. */
  catalogue(): Promise<Catalogue> {
    return this.call<Catalogue>({ type: 'catalogue' })
  }

  /** Loads a file into the core, replacing one of the same name. */
  addFile(name: string, data: ArrayBuffer): Promise<LoadedFile> {
    return this.call<LoadedFile>({ type: 'add-file', name, data }, [data])
  }

  removeFile(name: string): Promise<void> {
    return this.call<void>({ type: 'remove-file', name })
  }

  /** Adjusts the project; `generated` is the time to show in the report. */
  adjust(project: ProjectSpec, generated: string): Promise<Adjusted> {
    return this.call<Adjusted>({ type: 'adjust', project, generated })
  }

  /** The last adjustment's results in a format, as one or two files. */
  exportResults(format: string, ellipseScale: number, now: string): Promise<ExportedFile[]> {
    return this.call<ExportedFile[]>({ type: 'export', format, ellipseScale, now })
  }

  /** The project as project.toml's text, for the CLI. */
  projectToml(project: ProjectSpec): Promise<string> {
    return this.call<string>({ type: 'project-toml', project })
  }

  /** A project's files as a zip, the same bytes the CLI makes. Hands the files' buffers to the worker. */
  zipExport(files: ProjectFileData[]): Promise<ArrayBuffer> {
    return this.call<ArrayBuffer>({ type: 'zip-export', files }, files.map((f) => f.data))
  }

  /** The files in a project zip, checked: none escapes the project or is damaged. */
  zipImport(data: ArrayBuffer): Promise<ProjectFileData[]> {
    return this.call<ProjectFileData[]>({ type: 'zip-import', data }, [data])
  }

  /** Checks a RINEX observation file, read in slices, with navigation files for elevation coverage. */
  checkRinex(file: File, nav: File[]): Promise<RinexChecked> {
    return this.call<RinexChecked>({ type: 'check-rinex', file, nav })
  }

  /** Loads a project the way the CLI does and reports every problem, by file. */
  checkProject(files: ProjectFileData[]): Promise<CheckedProject> {
    return this.call<CheckedProject>({ type: 'check-project', files }, files.map((f) => f.data))
  }
  /** Fetches what the project's occupations need into the download cache. */
  fetchData(files: ProjectFileData[], options: FetchOptions): Promise<FetchOutcome> {
    return this.call<FetchOutcome>({ type: 'fetch-data', files, options }, files.map((f) => f.data))
  }

}

export const core = new CoreClient()
