// Where a project lives (ARCHITECTURE.md 14.2). Chrome and Edge edit a folder
// on disk in place; other browsers keep projects in browser storage (OPFS)
// and move them in and out as files or zips.

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

export async function openBrowserStorage(name = 'default'): Promise<ProjectRoot> {
  const root = await navigator.storage.getDirectory()
  const projects = await root.getDirectoryHandle('projects', { create: true })
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
