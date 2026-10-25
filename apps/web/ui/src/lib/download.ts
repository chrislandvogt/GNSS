// Hands a file to the browser to save.

export function download(fileName: string, data: BlobPart, type = 'application/octet-stream') {
  const url = URL.createObjectURL(new Blob([data], { type }))
  const link = document.createElement('a')
  link.href = url
  link.download = fileName
  link.click()
  setTimeout(() => URL.revokeObjectURL(url), 60_000)
}
