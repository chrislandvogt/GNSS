// Where a project lives (ARCHITECTURE.md 14.2). Chrome and Edge edit a folder
// on disk in place; other browsers keep projects in browser storage (OPFS)
// and move them in and out as files or zips. Zips are made and checked by the
// core, so this only moves bytes between handles.

import type { ProjectFileData } from '../workers/protocol'

export interface ProjectRoot {
  kind: 'folder' | 'browser'
  name: string
  handle: FileSystemDirectoryHandle
}

export interface ProjectFile {
  path: string
  size: number
  handle: FileSystemFileHandle
}

type DirectoryPicker = (options?: { id?: string; mode?: 'read' | 'readwrite' }) => Promise<FileSystemDirectoryHandle>

export const canOpenFolders = 'showDirectoryPicker' in window

export async function openFolder(): Promise<ProjectRoot> {
  const picker = (window as unknown as { showDirectoryPicker: DirectoryPicker }).showDirectoryPicker
  const handle = await picker({ id: 'gnss-project', mode: 'readwrite' })
  return { kind: 'folder', name: handle.name, handle }
}

async function browserProjects(): Promise<FileSystemDirectoryHandle> {
  const root = await navigator.storage.getDirectory()
  return root.getDirectoryHandle('projects', { create: true })
}

/** The projects in browser storage, by name. */
export async function listBrowserProjects(): Promise<string[]> {
  const names: string[] = []
  for await (const [name, entry] of (await browserProjects()).entries()) if (entry.kind === 'directory') names.push(name)
  return names.sort((a, b) => a.localeCompare(b))
}

export async function openBrowserStorage(name = 'default'): Promise<ProjectRoot> {
  const projects = await browserProjects()
  const handle = await projects.getDirectoryHandle(name, { create: true })
  // Browser storage can be evicted; ask the browser to keep it. The folder on
  // disk or an exported zip is still the durable copy.
  await navigator.storage.persist?.()
  return { kind: 'browser', name: `${name} (browser storage)`, handle }
}

/** Copy chosen files into a browser-storage project. */
export async function importFiles(project: ProjectRoot, files: FileList): Promise<void> {
  for (const file of Array.from(files)) {
    const target = await project.handle.getFileHandle(file.name, { create: true })
    const writable = await target.createWritable()
    await writable.write(file)
    await writable.close()
  }
}

/** Every file in the project, recursively, skipping the derived-data cache. */
export async function listFiles(dir: FileSystemDirectoryHandle, prefix = ''): Promise<ProjectFile[]> {
  const files: ProjectFile[] = []
  for await (const [name, entry] of dir.entries()) {
    const path = prefix ? `${prefix}/${name}` : name
    if (entry.kind === 'directory') {
      if (name !== '.cache' && name !== '.git') files.push(...(await listFiles(entry as FileSystemDirectoryHandle, path)))
    } else {
      const handle = entry as FileSystemFileHandle
      files.push({ path, size: (await handle.getFile()).size, handle })
    }
  }
  return files.sort((a, b) => a.path.localeCompare(b.path))
}

/** Every file in the project with its contents, for a zip or a check. */
export async function readProject(project: ProjectRoot): Promise<ProjectFileData[]> {
  const files = await listFiles(project.handle)
  return Promise.all(files.map(async (f) => ({ path: f.path, data: await (await f.handle.getFile()).arrayBuffer() })))
}

/**
 * A new project in browser storage holding the files from a zip, named after
 * the zip. An existing project is never overwritten: its raw files may be the
 * only copy.
 */
export async function createBrowserProject(name: string, files: ProjectFileData[]): Promise<ProjectRoot> {
  const clean = name.replace(/\.zip$/i, '').replace(/[^\w.-]+/g, '-').replace(/^[.-]+/, '') || 'project'
  if ((await listBrowserProjects()).includes(clean)) {
    throw new Error(`a project called ${clean} is already in browser storage; export and remove it first, or rename the zip`)
  }
  const project = await openBrowserStorage(clean)
  for (const file of files) {
    const parts = file.path.split('/')
    let dir = project.handle
    for (const part of parts.slice(0, -1)) dir = await dir.getDirectoryHandle(part, { create: true })
    const writable = await (await dir.getFileHandle(parts[parts.length - 1], { create: true })).createWritable()
    await writable.write(file.data)
    await writable.close()
  }
  return project
}

/** Removes a project from browser storage, after it has been exported. */
export async function removeBrowserProject(name: string): Promise<void> {
  await (await browserProjects()).removeEntry(name, { recursive: true })
}

/** How much browser storage is in use and allowed, and whether it's kept. */
export async function storageUse(): Promise<{ used: number; quota: number; persisted: boolean } | null> {
  if (!navigator.storage?.estimate) return null
  const { usage = 0, quota = 0 } = await navigator.storage.estimate()
  return { used: usage, quota, persisted: (await navigator.storage.persisted?.()) ?? false }
}

/** Writes a text file at the top of the project, replacing it if it's there. */
export async function writeProjectFile(project: ProjectRoot, name: string, text: string): Promise<void> {
  const writable = await (await project.handle.getFileHandle(name, { create: true })).createWritable()
  await writable.write(text)
  await writable.close()
}
