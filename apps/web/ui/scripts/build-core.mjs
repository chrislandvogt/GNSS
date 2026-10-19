// Compile the core (apps/web/core) to WebAssembly and generate its JavaScript
// bindings into src/generated/core/. Needs the wasm-bindgen CLI at the same
// version as the wasm-bindgen crate in Cargo.lock.
import { execFileSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'
import path from 'node:path'

const ui = path.dirname(path.dirname(fileURLToPath(import.meta.url)))
const root = path.resolve(ui, '../../..')
const profile = 'release-web'
const run = (cmd, args) => execFileSync(cmd, args, { cwd: root, stdio: 'inherit' })

run('cargo', ['build', '-p', 'gnss-web-core', '--target', 'wasm32-unknown-unknown', '--profile', profile, '--locked'])
run('wasm-bindgen', [
  '--target', 'web',
  '--out-dir', path.join(ui, 'src/generated/core'),
  path.join(root, 'target/wasm32-unknown-unknown', profile, 'gnss_web_core.wasm'),
])
