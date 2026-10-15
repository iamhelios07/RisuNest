import { readFile, writeFile } from 'node:fs/promises'
import { execFileSync } from 'node:child_process'
import { createRequire } from 'node:module'
import { webcrypto, createHash } from 'node:crypto'
import { isDeepStrictEqual } from 'node:util'
import ts from 'typescript'
import pako from 'pako'

const reference = process.argv[2] ?? process.env.RISUAI_REFERENCE_DIR
if (!reference) throw new Error('Pass the RisuAI reference checkout path as the first argument')
const require = createRequire(import.meta.url)
const output = new URL('../../src-tauri/src/native_file_jobs/legacy_backup/fixtures/', import.meta.url)
const compile = (source) => ts.transpileModule(source, {
    compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022, esModuleInterop: true },
}).outputText
const execute = (source, platform) => {
    const exports = {}
    new Function('require', 'exports', 'Buffer', compile(source))((name) => {
        if (name === './globalApi') return { isTauri: platform }
        if (name === 'lodash') return { isEqual: isDeepStrictEqual }
        return require(name)
    }, exports, Buffer)
    return exports
}
const git = (...args) => execFileSync('git', ['-C', reference, ...args], { encoding: 'utf8' }).trim()
const readWriter = (commit, path) => git('show', `${commit}:${path}`)
const fixtureDatabase = {
    username: 'Historical synthetic library',
    characters: [{ chaId: 'historical-character', type: 'character', name: 'Synthetic', chats: [
        { id: 'historical-chat', name: 'Chat', message: [{ role: 'user', data: '한국어 payload', chatId: 'historical-message' }] },
    ] }],
    botPresets: [{ id: 'historical-preset', name: 'Preset', temperature: 0.5 }],
    modules: [], plugins: [], loadouts: [], pluginCustomStorage: { synthetic: { value: 'kept' } },
    account: { token: 'synthetic-only' },
    historicalUnknown: { array: [null, true, 2, 'kept'], nested: { value: 'preserved' }, binary: new Uint8Array([0, 1, 127, 128, 255]) },
}
const formats = []
for (const [tag, commit] of [
    ['v1.27.2', '45094782e586522845744e5fb5ae898433a1cb69'],
    ['v1.27.3', '146a40872a919714f28436246ecedc4ce82fda3b'],
]) {
    const path = 'src/ts/storage/risuSave.ts'
    const source = readWriter(commit, path)
    for (const platform of [false, true]) {
        const bytes = execute(source, platform).encodeRisuSave(fixtureDatabase)
        formats.push({ tag, commit, path, native: platform, bytes: Buffer.from(bytes).toString('base64') })
    }
}
{
    const tag = 'v1.26.0'
    const commit = '89015ac00935f2cb6eda4b293f0360bd05b7b8ea'
    const path = 'src/ts/drive/drive.ts'
    const source = readWriter(commit, path)
    const ast = ts.createSourceFile(path, source, ts.ScriptTarget.Latest, true)
    let expression
    const visit = (node) => {
        if (ts.isVariableDeclaration(node) && node.name.getText(ast) === 'dbData'
            && node.initializer?.getText(ast).includes('Buffer.from(dbjson')) expression = node.initializer.getText(ast)
        ts.forEachChild(node, visit)
    }
    visit(ast)
    if (!expression) throw new Error('Released JSON backup writer was not found')
    const bytes = new Function('pako', 'Buffer', 'dbjson', `return (${expression})`)(pako, Buffer, JSON.stringify(fixtureDatabase))
    formats.push({ tag, commit, path, bytes: Buffer.from(bytes).toString('base64') })
}
// Use the product WebCrypto writer independently of the native implementation.
globalThis.window = { crypto: webcrypto }
const { encryptBuffer } = execute(await readFile(new URL('../../src/ts/bufferCrypto.ts', import.meta.url), 'utf8'))
const key = 'synthetic backup key 한국어'
for (const format of formats) {
    format.encrypted = Buffer.from(await encryptBuffer(Buffer.from(format.bytes, 'base64'), key)).toString('base64')
}
await writeFile(new URL('historical-writers.json', output), JSON.stringify({ database: fixtureDatabase, key, formats }) + '\n')
const cases = []
for (const size of [0, 1, 15, 16, 17, 65535, 65536, 65537]) {
    const plain = Uint8Array.from({ length: size }, (_, i) => (i * 13 + 7) & 255)
    const encrypted = await encryptBuffer(plain, key)
    cases.push({ size, sha256: createHash('sha256').update(plain).digest('hex'), ciphertext: Buffer.from(encrypted).toString('base64') })
}
await writeFile(new URL('account-encryption.json', output), JSON.stringify({ key, cases }) + '\n')
