import { readFile, writeFile, mkdir } from 'node:fs/promises'
import { fileURLToPath } from 'node:url'
import path from 'node:path'
import { createHash } from 'node:crypto'
import binaryen from 'binaryen'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..')
const input = await readFile(path.join(root, 'node_modules/wasmoon/dist/glue.wasm'))
if (createHash('sha256').update(input).digest('hex') !== '95f3f19ddb740125883bc41d5ec670cd0828e7b58c8bdad385323cbff497b55c') {
    throw new Error('Unexpected Wasmoon WASM. Install the pinned lockfile before building.')
}
const module = binaryen.readBinary(input)
module.setFeatures(binaryen.Features.MutableGlobals | binaryen.Features.SignExt | binaryen.Features.BulkMemory | binaryen.Features.BulkMemoryOpt)
module.runPasses(['asyncify'])
binaryen.setOptimizeLevel(3)
binaryen.setShrinkLevel(0)
module.optimize()
if (!module.validate()) throw new Error('Invalid Lua Worker WASM')
const output = path.join(root, 'src/ts/process/lua/worker.wasm')
const bytes = Buffer.from(module.emitBinary())
if (process.argv.includes('--check')) {
    if (!bytes.equals(await readFile(output))) throw new Error('Lua Worker WASM is stale; run pnpm lua:build-worker')
} else {
    await mkdir(path.dirname(output), { recursive: true })
    await writeFile(output, bytes)
}
module.dispose()
console.log(JSON.stringify({ bytes: bytes.length, sha256: createHash('sha256').update(bytes).digest('hex') }))
