// Messages between the main thread and the workers. The main thread only
// renders; the core and engines run in workers (ARCHITECTURE.md 14.1).

export interface CoreInfo {
  version: string
  build: string
}

// The shapes below mirror the core's types (apps/web/core and
// gnss_api::project) until types are generated from Rust (ARCHITECTURE.md
// section 8).

/** What the page offers: frames, import formats and export formats. */
export interface Catalogue {
  frames: { name: string; static: boolean }[]
  formats: { name: string; label: string; states_frame: boolean }[]
  exports: { name: string; label: string; description: string }[]
}

/** A file loaded into the core, with the format it looks like. */
export interface LoadedFile {
  name: string
  bytes: number
  sha256: string
  format: string | null
}

/** A project, in project.toml's shape. Imports name loaded files as raw/<name>. */
export interface ProjectSpec {
  schema: 1
  name: string
  adjustment: {
    frame: string
    epoch?: number
    datum: 'constrained' | 'minimal'
    uncertainty: 'a-priori' | 'a-posteriori'
  }
  import: { file: string; format: string; frame?: string; epoch?: string; base?: string; rover?: string }[]
  edit: { observation: string; action: 'exclude'; reason: string }[]
}

export interface Suspect {
  observation: string
  element: number
  label: string
  test: string
  statistic: number
  critical: number
  excess: number
}

export interface Summary {
  name: string
  frame: string
  epoch: number | null
  marks: number
  observations: number
  degrees_of_freedom: number
  variance_factor: number | null
  global_test: { statistic: number; lower: number; upper: number; passed: boolean } | null
  suspects: Suspect[]
  loops: [number, number] | null
  notes: string[]
  passed: boolean
}

export interface Adjusted {
  summary: Summary
  /** The HTML report, one self-contained page. */
  report: string
}

/** A file in a project, by its path relative to the project folder. */
export interface ProjectFileData {
  path: string
  data: ArrayBuffer
}

/** What checking a project found (apps/web/core's `Checked`). */
export interface CheckedProject {
  name: string
  marks: number
  occupations: number
  overlays: number
  issues: { file: string; message: string }[]
}

export interface ExportedFile {
  name: string
  data: ArrayBuffer
}

export type CoreRequest =
  | { id: number; type: 'info' }
  | { id: number; type: 'hash'; data: ArrayBuffer }
  | { id: number; type: 'catalogue' }
  | { id: number; type: 'add-file'; name: string; data: ArrayBuffer }
  | { id: number; type: 'remove-file'; name: string }
  | { id: number; type: 'adjust'; project: ProjectSpec; generated: string }
  | { id: number; type: 'export'; format: string; ellipseScale: number; now: string }
  | { id: number; type: 'project-toml'; project: ProjectSpec }
  | { id: number; type: 'zip-export'; files: ProjectFileData[] }
  | { id: number; type: 'zip-import'; data: ArrayBuffer }
  | { id: number; type: 'check-project'; files: ProjectFileData[] }

export type CoreResponse =
  | { id: number; ok: true; result: unknown }
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
