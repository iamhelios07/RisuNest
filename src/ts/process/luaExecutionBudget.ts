import { LUA_REGISTRYINDEX, LuaEventMasks, LuaReturn, type LuaEngine, type LuaWasm } from 'wasmoon'

const DEFAULT_TIMEOUT_MS = 5_000
const INSTRUCTION_HOOK_COUNT = 1_000

type EngineBudget = {
    timeoutMs: number
    hook: number
    formatter: number
}

const budgets = new WeakMap<LuaWasm, LuaExecutionBudget>()

export function getLuaExecutionBudget(lua: LuaWasm): LuaExecutionBudget {
    let budget = budgets.get(lua)
    if (!budget) {
        budget = new LuaExecutionBudget(lua)
        budgets.set(lua, budget)
    }
    return budget
}

// Wasmoon 1.16 shares a function table across engines. Coroutine hooks must outlive
// temporary JS call threads, so callbacks belong to the engine, not a single call.
class LuaExecutionBudget {
    private readonly owners = new Map<number, EngineBudget>()
    private readonly sourceDeadlines = new Map<number, number>()
    private readonly deadlines: number[] = []
    private readonly originalToString: LuaWasm['luaL_tolstring']

    constructor(private readonly lua: LuaWasm) {
        const resume = lua.lua_resume.bind(lua)
        lua.lua_resume = (address, ...args) => this.enter(address, () => resume(address, ...args))
        const pcall = lua.lua_pcallk.bind(lua)
        lua.lua_pcallk = (address, ...args) => this.enter(address, () => pcall(address, ...args))
        const call = lua.lua_callk.bind(lua)
        lua.lua_callk = (address, ...args) => this.enter(address, () => call(address, ...args))

        this.originalToString = lua.luaL_tolstring.bind(lua)
        // Wasmoon's error conversion can invoke __tostring outside a protected call.
        lua.luaL_tolstring = (address, index, length) => {
            const owner = this.owner(address)
            if (!owner) return this.originalToString(address, index, length)

            const absoluteIndex = lua.lua_absindex(address, index)
            lua.lua_pushcclosure(address, owner.formatter, 0)
            lua.lua_pushvalue(address, absoluteIndex)
            // Preserve an existing error even when source execution has expired.
            // Lua formatters still run under the inherited instruction deadline.
            const status = this.enter(address, () => pcall(address, 1, 1, 0, 0, null), false)
            if (status !== LuaReturn.Ok) {
                const failure = lua.lua_tolstring(address, -1, null)
                    ?? 'Lua error object could not be converted to text'
                lua.lua_pop(address, 1)
                lua.lua_pushstring(address, failure)
            }
            return lua.lua_tolstring(address, -1, length)
        }
    }

    private owner(address: number): EngineBudget | undefined {
        return this.owners.get(this.lua.lua_topointer(address, LUA_REGISTRYINDEX))
    }

    private enter<T>(address: number, run: () => T, checkSourceDeadline = true): T {
        const owner = this.owner(address)
        if (!owner) return run()

        const now = performance.now()
        const sourceDeadline = this.sourceDeadlines.get(address)
        // Frequent yields may never reach the instruction hook between resumes.
        if (checkSourceDeadline && sourceDeadline !== undefined && now > sourceDeadline) {
            throw new Error('thread timeout exceeded')
        }
        const deadline = Math.min(
            sourceDeadline ?? now + owner.timeoutMs,
            this.deadlines.at(-1) ?? Infinity,
        )
        this.lua.lua_sethook(address, owner.hook, LuaEventMasks.Count, INSTRUCTION_HOOK_COUNT)
        this.deadlines.push(deadline)
        try {
            return run()
        } finally {
            this.deadlines.pop()
        }
    }

    attach(engine: LuaEngine, timeoutMs = DEFAULT_TIMEOUT_MS): void {
        if (engine.global.isClosed()) throw new Error('Cannot attach a closed Lua engine')
        const key = this.lua.lua_topointer(engine.global.address, LUA_REGISTRYINDEX)
        if (this.owners.has(key)) return

        const hook = this.lua.module.addFunction((address: number) => {
            const deadline = this.deadlines.at(-1)
            if (deadline !== undefined && performance.now() > deadline) {
                this.lua.lua_pushstring(address, 'thread timeout exceeded')
                this.lua.lua_error(address)
            }
        }, 'vii')
        let formatter: number
        try {
            formatter = this.lua.module.addFunction((address: number) => {
                this.originalToString(address, 1, null)
                return 1
            }, 'ii')
        } catch (error) {
            this.lua.module.removeFunction(hook)
            throw error
        }
        this.owners.set(key, { timeoutMs, hook, formatter })
        const close = engine.global.close.bind(engine.global)
        let closed = false
        engine.global.close = () => {
            if (closed) return
            closed = true
            try {
                close()
            } finally {
                this.owners.delete(key)
                this.lua.module.removeFunction(hook)
                this.lua.module.removeFunction(formatter)
            }
        }
    }

    async runSource(engine: LuaEngine, source: string, timeoutMs = DEFAULT_TIMEOUT_MS): Promise<void> {
        this.attach(engine)
        const thread = engine.global.newThread()
        const threadIndex = engine.global.getTop()
        try {
            thread.loadString(source)
            // Source evaluation keeps its deadline across awaits; handlers get a
            // new budget on each host entry, without extending a reentrant call.
            this.sourceDeadlines.set(thread.address, performance.now() + timeoutMs)
            await thread.run()
        } finally {
            this.sourceDeadlines.delete(thread.address)
            try {
                thread.close()
            } finally {
                engine.global.remove(threadIndex)
            }
        }
    }
}
