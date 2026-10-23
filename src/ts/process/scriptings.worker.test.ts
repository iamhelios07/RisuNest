// @vitest-environment node

import { readFile } from 'node:fs/promises'
import { afterAll, beforeAll, expect, test, vi } from 'vitest'
import { nodeLuaWorkers } from './tests/luaWorkerNode'
import type { Chat } from '../storage/database.svelte'
import { LuaWorkerExecutor } from './luaWorkerExecutor'
import { luaCodeWrapper } from './luaCodeWrapper'

vi.mock('../storage/persistentDataRuntime.svelte', async () => (await import('./tests/scriptingsTestHarness')).persistentRuntimeModule())
vi.mock('../parser/parser.svelte', async () => (await import('./tests/scriptingsTestHarness')).parserModule())
vi.mock('../alert', async () => (await import('./tests/scriptingsTestHarness')).alertModule())
vi.mock('../globalApi.svelte', () => ({ fetchNative: vi.fn(), readImage: vi.fn() }))
vi.mock('../platform', async () => (await import('../platform.testSupport')).platformModule({ runtime: 'tauri', os: 'android' }))
vi.mock('../tokenizer', async () => (await import('./tests/scriptingsTestHarness')).tokenizerModule())
vi.mock('../util', async () => (await import('./tests/scriptingsTestHarness')).utilModule())
vi.mock('../storage/database.svelte', async () => (await import('./tests/scriptingsTestHarness')).databaseModule())
vi.mock('../stores.svelte', async () => (await import('./tests/scriptingsTestHarness')).storesModule())
vi.mock('./modules', async () => (await import('./tests/scriptingsTestHarness')).modulesModule())
vi.mock('./files/inlays', async () => (await import('./tests/scriptingsTestHarness')).inlaysModule())
vi.mock('./lorebook.svelte', async () => (await import('./tests/scriptingsTestHarness')).lorebookModule())
vi.mock('./memory/hypamemory', async () => (await import('./tests/scriptingsTestHarness')).hypamemoryModule())
vi.mock('./request/request', async () => (await import('./tests/scriptingsTestHarness')).requestModule())
vi.mock('./stableDiff', async () => (await import('./tests/scriptingsTestHarness')).stableDiffModule())
vi.mock('./command', async () => (await import('./tests/scriptingsTestHarness')).commandModule())

let runScripted: typeof import('./scriptings').runScripted
let workers: Awaited<ReturnType<typeof nodeLuaWorkers>>
beforeAll(async () => {
    workers = await nodeLuaWorkers()
    vi.stubGlobal('Worker', workers.Worker)
    const json = await readFile('public/lua/json.lua', 'utf8')
    vi.stubGlobal('fetch', vi.fn(async () => new Response(json)))
    runScripted = (await import('./scriptings')).runScripted
})
afterAll(async () => { await workers.close(); vi.unstubAllGlobals() })

test('runs real host APIs in C callbacks, preserves independent views and read-after-write', async () => {
    const chat = { message: [{ role: 'user', data: 'before', chatId: 'a' }] } as Chat
    const result = await runScripted(`function onStart(id)
        local a = getFullChat(id)
        a[1].data = 'local'
        assert(getFullChat(id)[1].data == 'before')
        local first = ('x'):gsub('x', function() return getChatData(id, 0) end)
        setChat(id, 0, 'after')
        return first .. ':' .. getFullChat(id)[1].data
    end`, { char: { chaId: 'worker-read-write' } as never, chat, mode: 'start' })
    expect(result.res).toBe('before:after')
    expect(chat.message[0].data).toBe('after')
})

test('preserves explicit false, nil, missing handlers and retained globals', async () => {
    const arg = { char: { chaId: 'worker-results' } as never, chat: { message: [] } as Chat, mode: 'start' }
    const code = 'counter=0; function onStart() counter=counter+1; return counter end'
    expect((await runScripted(code, arg)).res).toBe(1)
    expect((await runScripted(code, arg)).res).toBe(2)
    expect((await runScripted(code, { ...arg, char: { chaId: 'worker-other' } as never })).res).toBe(1)
    expect((await runScripted('function onStart() return false end', arg)).stopSending).toBe(true)
    expect((await runScripted('function onStart() end', arg)).res).toBeNull()
    expect((await runScripted('', arg)).res).toBeUndefined()
})

test('retains the existing async host shape and display write restrictions', async () => {
    const chat = { message: [{ role: 'user', data: 'before' }] } as Chat
    const vars = new Map<string, string>()
    const result = await runScripted(`listenEdit('editDisplay', function(id, text)
        setChat(id, 0, 'forbidden')
        setState(id, 'x', {n=1})
        return text .. ':' .. getState(id, 'x').n .. ':' .. getChatData(id, 0)
    end)`, { char: { chaId: 'worker-display' } as never, chat, data: 'display', mode: 'editDisplay',
        setVar: (key, value) => { vars.set(key, value) }, getVar: key => vars.get(key) ?? 'null' })
    expect(result.res).toBe('display:1:before')
    expect(chat.message[0].data).toBe('before')
    const waited = await runScripted(`onOutput=async(function(id)
        sleep(id, 1):await()
        return getChatData(id, 0)
    end)`, { char: { chaId: 'worker-async' } as never, chat, mode: 'output' })
    expect(waited.res).toBe('before')
})

test('does not renew a handler deadline on synchronous RPC and terminates a suspended failed engine', async () => {
    const executor = new LuaWorkerExecutor(30)
    executor.register('ping', () => 1)
    try {
        await executor.initialize(luaCodeWrapper('function run() while true do ping() end end'), await readFile('public/lua/json.lua', 'utf8'))
        await expect(executor.invoke('run', [])).rejects.toThrow(/thread timeout exceeded/)
        expect(executor.isClosed()).toBe(true)
    } finally { executor.close() }
}, 10_000)

test('keeps the handler deadline separate from a genuine async wait', async () => {
    const executor = new LuaWorkerExecutor(30)
    let finish!: (value: string) => void
    let started!: () => void
    const ready = new Promise<void>(resolve => { started = resolve })
    executor.register('wait', () => { started(); return new Promise<string>(resolve => { finish = resolve }) })
    try {
        await executor.initialize(luaCodeWrapper('onOutput=async(function() return wait():await() end)'), await readFile('public/lua/json.lua', 'utf8'))
        const result = executor.invoke('onOutput', [])
        await ready
        await new Promise(resolve => setTimeout(resolve, 60))
        finish('completed')
        expect(await result).toBe('completed')
    } finally { executor.close() }
})

test('aborts a pending invocation without replaying completed effects or running its late continuation', async () => {
    const executor = new LuaWorkerExecutor(1_000)
    const other = new LuaWorkerExecutor(1_000)
    const writes: string[] = []
    let finish!: () => void
    let started!: () => void
    const ready = new Promise<void>(resolve => { started = resolve })
    executor.register('write', value => { writes.push(value) })
    executor.register('wait', () => { started(); return new Promise<void>(resolve => { finish = resolve }) })
    const controller = new AbortController()
    try {
        await other.initialize('count=0; function run() count=count+1; return count end', await readFile('public/lua/json.lua', 'utf8'))
        expect(await other.invoke('run', [])).toBe(1)
        await executor.initialize(luaCodeWrapper(`onOutput=async(function()
            write('before'); wait():await(); write('after')
        end)`), await readFile('public/lua/json.lua', 'utf8'))
        const result = executor.invoke('onOutput', [], controller.signal)
        const failed = expect(result).rejects.toThrow('cancelled')
        await ready
        controller.abort(new Error('cancelled'))
        await failed
        finish()
        await Promise.resolve()
        expect(executor.isClosed()).toBe(true)
        expect(writes).toEqual(['before'])
        expect(await other.invoke('run', [])).toBe(2)
        expect(other.isClosed()).toBe(false)
    } finally { executor.close(); other.close() }
})

test('does not let a detached old Promise callback access a later invocation', async () => {
    const executor = new LuaWorkerExecutor(1_000)
    const writes: string[] = []
    let finish!: () => void
    executor.register('write', value => { writes.push(value) })
    executor.register('wait', () => new Promise<void>(resolve => { finish = resolve }))
    try {
        await executor.initialize(luaCodeWrapper(`
            function detach()
                pending=wait():next(function() write('stale') end):catch(function() end)
                return 'detached'
            end
            onOutput=async(function()
                local ok=pcall(function() pending:await() end)
                write('current')
                return ok
            end)
        `), await readFile('public/lua/json.lua', 'utf8'))
        expect(await executor.invoke('detach', [])).toBe('detached')
        const result = executor.invoke('onOutput', [])
        finish()
        await result
        expect(writes).toEqual(['current'])
    } finally { executor.close() }
})

test('keeps pure Promise continuations retained across invocations', async () => {
    const executor = new LuaWorkerExecutor(1_000)
    let finish!: (value: string) => void
    executor.register('wait', () => new Promise<string>(resolve => { finish = resolve }))
    try {
        await executor.initialize(luaCodeWrapper(`
            function detach()
                pending=wait():next(function(value) return value..':continued' end)
                return 'detached'
            end
            onOutput=async(function() return pending:await() end)
        `), await readFile('public/lua/json.lua', 'utf8'))
        expect(await executor.invoke('detach', [])).toBe('detached')
        const result=executor.invoke('onOutput', [])
        finish('original')
        expect(await result).toBe('original:continued')
    } finally { executor.close() }
})

test('formats error objects through synchronous host calls inside the protected Lua execution', async () => {
    const executor = new LuaWorkerExecutor(1_000)
    executor.register('describe', () => 'synthetic error')
    try {
        await executor.initialize(luaCodeWrapper(`
            local failure=setmetatable({}, {__tostring=function() return describe() end})
            function direct() error(failure) end
            onOutput=async(function()
                return Promise.resolve('ready'):next(function() error(failure) end):await()
            end)
        `), await readFile('public/lua/json.lua', 'utf8'))
        await expect(executor.invoke('direct', [])).rejects.toThrow('synthetic error')
        await expect(executor.invoke('onOutput', [])).rejects.toThrow('synthetic error')
    } finally { executor.close() }
}, 10_000)

test('keeps host Error userdata and primitive rejection values compatible', async () => {
    const executor = new LuaWorkerExecutor(1_000)
    executor.register('fail', () => { throw new Error('host problem') })
    executor.register('reject', () => Promise.reject('primitive rejection'))
    try {
        await executor.initialize(luaCodeWrapper(`
            function direct()
                local ok, problem=pcall(fail)
                return {ok=ok, message=problem.message, hidden=problem.constructor==null}
            end
            onOutput=async(function() return reject():await() end)
        `), await readFile('public/lua/json.lua', 'utf8'))
        expect(await executor.invoke('direct', [])).toEqual({ok:false, message:'host problem', hidden:true})
        await expect(executor.invoke('onOutput', [])).rejects.toBe('primitive rejection')
    } finally { executor.close() }
})

test('propagates product cancellation and keeps only effects completed before the wait', async () => {
    const { alertInput } = await import('../alert')
    let finish!: (value: string) => void
    let started!: () => void
    const ready = new Promise<void>(resolve => { started = resolve })
    vi.mocked(alertInput).mockImplementationOnce(() => {
        started()
        return new Promise<string>(resolve => { finish = resolve })
    })
    const chat = { message: [{ role: 'user', data: 'original' }] } as Chat
    const controller = new AbortController()
    const result = runScripted(`onOutput=async(function(id)
        setChat(id, 0, 'before')
        alertInput(id, 'synthetic'):await()
        setChat(id, 0, 'after')
    end)`, { char: { chaId: 'worker-product-abort' } as never, chat, mode: 'output', signal: controller.signal })
    const rejected = expect(result).rejects.toThrow('cancelled')
    await ready
    controller.abort(new Error('cancelled'))
    try { await rejected }
    finally { finish('late') }
    expect(chat.message[0].data).toBe('before')
})

test('recovers on the next request after a Worker cannot run its instruction hook', async () => {
    const executor = new LuaWorkerExecutor(30)
    try {
        await executor.initialize('function run() debug.sethook(); while true do end end', await readFile('public/lua/json.lua', 'utf8'))
        await expect(executor.invoke('run', [])).rejects.toThrow('thread timeout exceeded')
        expect(executor.isClosed()).toBe(true)
        const next = new LuaWorkerExecutor(1_000)
        try {
            await next.initialize('function run() return "recovered" end', await readFile('public/lua/json.lua', 'utf8'))
            expect(await next.invoke('run', [])).toBe('recovered')
        } finally { next.close() }
    } finally { executor.close() }
}, 15_000)
