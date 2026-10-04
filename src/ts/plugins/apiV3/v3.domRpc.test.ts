import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { SandboxHost } from './factory'
import {
    createProductionPluginDatabaseAccess,
    linkPluginQueryAbortSignals,
} from '../pluginDatabaseAccess'
import {
    executePluginV3,
    getV3PluginInstance,
    loadV3Plugins,
} from './v3.svelte'

const mocks = vi.hoisted(() => ({
    database: null as any,
    selectedId: 0,
    confirm: vi.fn(async () => true),
    permissionValues: new Map<string, unknown>(),
    providers: new Map<string, Function>(),
}))

const ownedStorageStub = {
    getItem: vi.fn(), setItem: vi.fn(), removeItem: vi.fn(), clear: vi.fn(),
    key: vi.fn(), keys: vi.fn(), length: vi.fn(), snapshot: vi.fn(async () => ({})),
    mutate: vi.fn(),
}

vi.mock('../plugins.svelte', () => {
    const oldApis = new Proxy({}, { get: () => vi.fn() })
    return {
        allowedDbKeys: [],
        applyPreparedPluginDatabaseUpdate: vi.fn(),
        chatOutputListenerProvenance: new WeakMap(),
        customProviderStore: {
            subscribe: (run: (value: unknown) => void) => { run([]); return () => undefined },
            set: vi.fn(),
            update: vi.fn(),
        },
        getV2PluginAPIs: () => oldApis,
        handlePluginInstallViaPlugin: vi.fn(),
        pluginCompatibility: { profile: 'scalable-v3' },
        pluginStorageStore: {
            forOwner: () => ownedStorageStub,
            ownerOf: () => 'test-plugin',
            invalidateOwner: vi.fn(),
            synchronizeCommittedMutation: vi.fn(),
            snapshot: vi.fn(), mutate: vi.fn(), invalidate: vi.fn(), getItem: vi.fn(),
            setItem: vi.fn(), removeItem: vi.fn(), clear: vi.fn(), key: vi.fn(),
            keys: vi.fn(), length: vi.fn(),
        },
        pluginV2: { providers: mocks.providers, providerOptions: new Map() },
    }
})
vi.mock('src/ts/storage/database.svelte', () => ({ getDatabase: () => mocks.database }))
vi.mock('../pluginSafeClass', () => ({
    SafeLocalPluginStorage: class {},
    SafeLocalStorage: class {},
    tagWhitelist: ['button', 'span', 'div'],
}))
vi.mock('src/ts/stores.svelte', () => ({
    DBState: {
        get db() { return mocks.database },
        set db(value) { mocks.database = value },
    },
    selectedCharID: {
        subscribe(run: (value: number) => void) { run(mocks.selectedId); return () => undefined },
    },
    additionalChatMenu: [], additionalFloatingActionButtons: [], additionalHamburgerMenu: [],
    additionalSettingsMenu: [], bodyIntercepterStore: [], chatPanelStore: [],
}))
vi.mock('src/ts/alert', () => ({
    alertConfirm: mocks.confirm,
    alertError: vi.fn(),
    alertNormal: vi.fn(),
}))
vi.mock('src/ts/util', () => ({ sleep: vi.fn(async () => undefined) }))
vi.mock('src/lang', () => ({ language: {
    fetchLogConsent: '{}', getFullDatabaseConsent: '{}', mainDomAccessConsent: '{}',
    replacerPermissionConsent: '{}', providerPermissionConsent: '{}', sendChatConsent: '{}',
    inlayPermissionConsent: '{}', pluginProviderPermissionDenied: 'permission denied',
} }))
vi.mock('src/ts/globalApi.svelte', () => ({
    checkCharOrder: vi.fn(), forageStorage: {}, getFetchLogs: vi.fn(),
}))
vi.mock('src/ts/gui/colorscheme', () => ({
    changeColorScheme: vi.fn(), updateColorScheme: vi.fn(), updateTextThemeAndCSS: vi.fn(),
}))
vi.mock('src/ts/platform', () => ({ isTauri: false }))
vi.mock('src/ts/process/mcp/pluginmcp', () => ({
    registerMCPModule: vi.fn(), unregisterMCPModule: vi.fn(),
}))
vi.mock('src/ts/process/files/inlays', () => ({ getInlayAsset: vi.fn() }))
vi.mock('src/ts/translator/translator', () => ({
    getLLMCache: vi.fn(), searchLLMCache: vi.fn(),
}))
vi.mock('src/ts/parser/parser.svelte', () => ({ hasher: vi.fn(async () => 'fixture-hash') }))
vi.mock('localforage', () => ({ default: { createInstance: () => ({
    getItem: vi.fn((key: string) => mocks.permissionValues.get(key) ?? null),
    setItem: vi.fn((key: string, value: unknown) => mocks.permissionValues.set(key, value)),
}) } }))
vi.mock('src/ts/process/index.svelte', () => ({
    sendChat: vi.fn(),
    doingChat: {
        subscribe(run: (value: boolean) => void) { run(false); return () => undefined },
    },
}))
vi.mock('src/ts/model/modellist', () => ({ getModelInfo: () => ({ id: 'fixture-model' }) }))
vi.mock('src/ts/process/request/request', () => ({ requestChatDataMain: vi.fn() }))
vi.mock('src/ts/process/modules', () => ({ getModuleLorebooks: vi.fn() }))
vi.mock('src/ts/process/ttsHooks', () => ({
    registerTTSPreprocessor: vi.fn(), unregisterTTSPreprocessor: vi.fn(),
    registerTTSPostprocessor: vi.fn(), unregisterTTSPostprocessor: vi.fn(),
}))
vi.mock('src/ts/storage/persistentDataRuntime.svelte', () => ({
    getPersistentRevision: () => 0,
    commitPersistentUnitIntent: vi.fn(),
    flushPendingDataLocally: vi.fn(), acquireCompleteConversation: vi.fn(),
    assertPersistentMutationAllowed: vi.fn(),
    getPersistentStorageAuthorityEpoch: () => 0,
    captureSelectedConversationTarget: vi.fn(), getActiveConversationSession: vi.fn(),
    getPersistentNavigationGeneration: vi.fn(() => 0),
    invalidateActiveConversationSession: vi.fn(),
    materializePersistentDatabaseSnapshotWithRevision: vi.fn(),
    refreshSelectedConversationAfterReplacement: vi.fn(),
    replacePersistentCompleteCharacter: vi.fn(),
    replacePersistentConversation: vi.fn(), replacePersistentDatabase: vi.fn(),
}))
vi.mock('../pluginCompatibility', () => ({ runPluginFullObjectReplacement: vi.fn() }))
vi.mock('../pluginDatabaseAccess', () => ({
    createProductionPluginDatabaseAccess: vi.fn(() => ({})),
    linkPluginQueryAbortSignals: vi.fn(),
}))
vi.mock('../pluginChatOutputListeners', () => ({
    registerChatOutputListener: vi.fn(), removeChatOutputListener: vi.fn(),
}))
vi.mock('src/ts/conversationMutations', () => ({ appendCurrentConversationMessage: vi.fn() }))

const pluginName = 'dom-rpc-fixture'
const happyDOMWindow = window as unknown as Window & {
    happyDOM: { settings: { enableJavaScriptEvaluation: boolean } }
}
let guestEvaluation = Promise.resolve<unknown>(undefined)
let guestEvaluations: Promise<unknown>[] = []

const fixtureScript = `
globalThis.rpcState = {
    targetClicks: 0,
    documentClicks: 0,
    mutationType: '',
    mutationTargetMatches: false,
    addedChildId: '',
};
globalThis.rpcReady = (async () => {
    await risuai.requestPluginPermission('provider');
    const root = await risuai.getRootDocument();
    if (!root) throw new Error('mainDom permission was unexpectedly denied');
    const target = await root.querySelector('#plugin-target');
    if (!target) throw new Error('fixture target is missing');

    await target.addEventListener('click', (event) => {
        if (event.type === 'click') globalThis.rpcState.targetClicks += 1;
    });
    await root.addEventListener('click', () => {
        globalThis.rpcState.documentClicks += 1;
    });

    const observer = await risuai.createMutationObserver(async (records) => {
        const record = await records.at(0);
        if (!record) throw new Error('mutation record is missing');
        const mutationTarget = await record.getTarget();
        const addedNodes = await record.getAddedNodes();
        const firstAdded = await addedNodes.at(0);
        globalThis.rpcState.mutationType = await record.getType();
        globalThis.rpcState.mutationTargetMatches = await mutationTarget.matches('#plugin-target');
        globalThis.rpcState.addedChildId = firstAdded
            ? await firstAdded.getAttribute('x-fixture-child') || ''
            : '';
    });
    await observer.observe(target, { childList: true });
    globalThis.fixtureTarget = target;
})();
`

function installFixtureDatabase(name = pluginName, script = fixtureScript): void {
    mocks.database = {
        aiModel: 'fixture-model',
        characters: [],
        plugins: [{ name, script }],
    }
}

async function guest(name: string, code: string): Promise<unknown> {
    const instance = getV3PluginInstance(name)
    if (!instance) throw new Error(`missing fixture plugin ${name}`)
    expect(instance.host).toBeInstanceOf(SandboxHost)
    return instance.host.executeInIframe(code)
}

async function waitFor<T>(read: () => Promise<T>, matches: (value: T) => boolean): Promise<T> {
    for (let attempt = 0; attempt < 50; attempt += 1) {
        const value = await read()
        if (matches(value)) return value
        await new Promise<void>((resolve) => setTimeout(resolve, 0))
    }
    throw new Error('fixture condition did not settle')
}

async function readState(name: string) {
    return JSON.parse(String(await guest(
        name,
        'await globalThis.rpcReady; return JSON.stringify(globalThis.rpcState)',
    ))) as {
        targetClicks: number
        documentClicks: number
        mutationType: string
        mutationTargetMatches: boolean
        addedChildId: string
    }
}

async function startFixture(name = pluginName, script = fixtureScript): Promise<void> {
    installFixtureDatabase(name, script)
    await executePluginV3({ name, script } as any)
    await guestEvaluation
    await guest(name, 'await globalThis.rpcReady; return true')
}

describe('Plugin v3 real iframe DOM/RPC bridge', () => {
    beforeEach(() => {
        vi.clearAllMocks()
        vi.stubGlobal('ImageBitmap', class {})
        const srcdocDescriptor = Object.getOwnPropertyDescriptor(HTMLIFrameElement.prototype, 'srcdoc')
        let frameSequence = 0
        vi.spyOn(HTMLIFrameElement.prototype, 'srcdoc', 'set').mockImplementation(function (value: string) {
            frameSequence += 1
            const frameId = `dom-rpc-frame-${frameSequence}`
            const transportShim = `
globalThis.ImageBitmap = class {};
function __postToParent(message) {
    const source = parent.document.querySelector('iframe[data-risu-test-frame="${frameId}"]').contentWindow;
    const data = JSON.parse(JSON.stringify(message));
    parent.dispatchEvent(new parent.MessageEvent('message', { data, source }));
}
`
            const sourceValue = value
                .replace(/window\.parent\.postMessage/g, '__postToParent')
                .replace(/parent\.postMessage/g, '__postToParent')
                .replace(/(<script nonce="[^"]+">)/, `$1${transportShim}`)
            const settings = happyDOMWindow.happyDOM.settings
            settings.enableJavaScriptEvaluation = false
            srcdocDescriptor?.set?.call(this, sourceValue)
            settings.enableJavaScriptEvaluation = true

            this.setAttribute('data-risu-test-frame', frameId)
            const child = this.contentWindow!
            const childRealm = child as any
            vi.spyOn(child, 'postMessage').mockImplementation((data) => {
                child.dispatchEvent(new childRealm.MessageEvent('message', {
                    data: structuredClone(data),
                    source: child.parent,
                }))
            })
            const source = sourceValue.match(/<script nonce="[^"]+">([\s\S]*)<\/script>/)?.[1]
            if (!source) throw new Error('Sandbox guest script was not found')
            guestEvaluation = childRealm.eval(source)
            guestEvaluations.push(guestEvaluation)
        })
        mocks.permissionValues.clear()
        mocks.providers.clear()
        guestEvaluations = []
        mocks.confirm.mockResolvedValue(true)
        mocks.selectedId = 0
        happyDOMWindow.happyDOM.settings.enableJavaScriptEvaluation = true
        document.body.replaceChildren()
        document.body.insertAdjacentHTML('beforeend', [
            '<button id="plugin-target">plugin</button>',
            '<button id="other-target">other</button>',
        ].join(''))
        installFixtureDatabase()
    })

    afterEach(async () => {
        await loadV3Plugins([])
        document.body.replaceChildren()
        vi.unstubAllEnvs()
        vi.restoreAllMocks()
    })

    it('returns method-specific icon errors through the real bridge without argument contents', async () => {
        await startFixture('icon-errors-fixture')
        const errors = JSON.parse(
            String(
                await guest(
                    'icon-errors-fixture',
                    `
            const errors = [];
            for (const call of [
                () => risuai.registerSetting('fixture', () => {}, '', 'secret-fixture'),
                () => risuai.registerButton({ name: 'fixture', icon: '', iconType: 'secret-fixture' }, () => {}),
                () => risuai.registerButton(null, () => {}),
            ]) {
                try { await call(); } catch (error) { errors.push(error.message); }
            }
            return JSON.stringify(errors);
        `,
                ),
            ),
        ) as string[]
        expect(errors[0]).toMatch(/^\[Plugin API: registerSetting\]/)
        expect(errors[0]).toContain(
            "registerSetting: fourth argument iconType must be 'html', 'img' or 'none'",
        )
        expect(errors[1]).toContain(
            "registerButton: options.iconType must be 'html', 'img' or 'none'",
        )
        expect(errors[2]).toContain(
            'registerButton: first argument must be an options object',
        )
        expect(errors.join()).not.toContain('secret-fixture')
    })

    it('identifies the originating plugin in uncaught guest code stacks', async () => {
        await startFixture(
            'stack-fixture',
            `
            globalThis.rpcReady = Promise.resolve();
            globalThis.throwFixture = () => { throw new TypeError('synthetic failure'); };
        `,
        )
        const stack = String(
            await guest(
                'stack-fixture',
                `
            try { globalThis.throwFixture(); } catch (error) { return error.stack; }
        `,
            ),
        )
        expect(stack).toContain('risu-plugin-v3/stack-fixture.js')
        expect(stack).toContain('TypeError: synthetic failure')
    })

    it('does not reuse a provider permission decision for mainDom', async () => {
        await startFixture()

        expect(mocks.confirm).toHaveBeenCalledTimes(2)
    })

    it('does not invoke a provider callback when provider permission is denied', async () => {
        const name = 'denied-provider-fixture'
        mocks.confirm.mockResolvedValue(false)
        await startFixture(name, `
            globalThis.providerCalls = 0;
            globalThis.rpcReady = risuai.addProvider('denied-provider', async () => {
                globalThis.providerCalls += 1;
                return { success: true, content: 'unexpected' };
            });
        `)

        const provider = mocks.providers.get('denied-provider')
        expect(provider).toBeTypeOf('function')
        const result = await provider!({
            prompt_chat: [{ role: 'user', content: 'sensitive sentinel' }],
            mode: 'chat',
        })

        expect(result).toEqual({ success: false, content: 'permission denied' })
        expect(await guest(name, 'return globalThis.providerCalls')).toBe(0)
    })

    it('binds a SafeElement listener to its element and keeps SafeDocument at document scope', async () => {
        await startFixture()
        document.querySelector('#other-target')?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
        document.querySelector('#plugin-target')?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
        document.dispatchEvent(new Event('click'))

        const state = await waitFor(
            () => readState(pluginName),
            (value) => value.targetClicks === 1 && value.documentClicks === 3,
        )
        expect(state).toMatchObject({ targetClicks: 1, documentClicks: 3 })
    })

    it('delivers MutationObserver records as callable remote proxies', async () => {
        await startFixture()
        const child = document.createElement('span')
        child.setAttribute('x-fixture-child', 'added')
        document.querySelector('#plugin-target')?.append(child)

        const state = await waitFor(
            () => readState(pluginName),
            (value) => value.addedChildId === 'added',
        )
        expect(state).toMatchObject({
            mutationType: 'childList',
            mutationTargetMatches: true,
            addedChildId: 'added',
        })
    })

    it('terminates both existing plugin iframes before a reload', async () => {
        await loadV3Plugins([
            { name: 'first-fixture', script: '' },
            { name: 'second-fixture', script: '' },
        ] as any)
        await Promise.all(guestEvaluations)
        expect(document.querySelectorAll('iframe[data-risu-plugin-frame]')).toHaveLength(2)

        await loadV3Plugins([])

        expect(document.querySelectorAll('iframe[data-risu-plugin-frame]')).toHaveLength(0)
    })

    describe('plugin channel IPC', () => {
        const ipcScript = (channel: string) => `
            globalThis.received = [];
            globalThis.rpcReady = risuai.addPluginChannelListener(${JSON.stringify(channel)}, (message, meta) => {
                globalThis.received.push({ message, sender: meta.sender, channel: meta.channel });
            });
        `

        async function startIpcPlugins(plugins: { name: string; allowedIPC: string[]; channel?: string }[]): Promise<void> {
            const records = plugins.map((plugin) => ({
                name: plugin.name,
                script: ipcScript(plugin.channel ?? 'inbox'),
                allowedIPC: plugin.allowedIPC,
            }))
            mocks.database = { aiModel: 'fixture-model', characters: [], plugins: records }
            await loadV3Plugins(records as any)
            await Promise.all(guestEvaluations)
            for (const plugin of plugins) await guest(plugin.name, 'await globalThis.rpcReady; return true')
        }

        async function post(sender: string, receiver: string, channel: string, message: unknown): Promise<unknown> {
            return guest(sender, `
                const result = await risuai.postPluginChannelMessage(${JSON.stringify(receiver)}, ${JSON.stringify(channel)}, ${JSON.stringify(message)});
                return result === undefined ? 'resolved' : 'unexpected';
            `)
        }

        async function received(name: string) {
            return JSON.parse(String(await guest(name, 'return JSON.stringify(globalThis.received)'))) as {
                message: unknown
                sender: string
                channel: string
            }[]
        }

        async function settle(): Promise<void> {
            for (let attempt = 0; attempt < 20; attempt += 1) {
                await new Promise<void>((resolve) => setTimeout(resolve, 0))
            }
        }

        it('delivers both ways between a wildcard plugin and a client that names it, with the host-attested sender', async () => {
            vi.spyOn(console, 'warn').mockImplementation(() => undefined)
            await startIpcPlugins([
                { name: 'hub', allowedIPC: ['hub', '*'] },
                { name: 'client', allowedIPC: ['client', 'hub'] },
            ])

            expect(await post('client', 'hub', 'inbox', { register: true })).toBe('resolved')
            expect(await post('hub', 'client', 'inbox', { reply: true })).toBe('resolved')

            expect(await waitFor(() => received('hub'), (value) => value.length === 1)).toEqual([
                { message: { register: true }, sender: 'client', channel: 'inbox' },
            ])
            expect(await waitFor(() => received('client'), (value) => value.length === 1)).toEqual([
                { message: { reply: true }, sender: 'hub', channel: 'inbox' },
            ])
        })

        it('keeps the two-sided rule when only one side lists the other or a wildcard', async () => {
            const warn = vi.spyOn(console, 'warn').mockImplementation(() => undefined)
            await startIpcPlugins([
                { name: 'hub', allowedIPC: ['hub', '*'] },
                { name: 'stranger', allowedIPC: ['stranger'] },
                { name: 'one-sided', allowedIPC: ['one-sided', 'plain'] },
                { name: 'plain', allowedIPC: ['plain'] },
            ])

            expect(await post('stranger', 'hub', 'inbox', 1)).toBe('resolved')
            expect(await post('hub', 'stranger', 'inbox', 2)).toBe('resolved')
            expect(await post('one-sided', 'plain', 'inbox', 3)).toBe('resolved')
            expect(await post('plain', 'one-sided', 'inbox', 4)).toBe('resolved')
            expect(await post('hub', 'missing-plugin', 'inbox', 5)).toBe('resolved')
            await settle()

            for (const name of ['hub', 'stranger', 'one-sided', 'plain']) {
                expect(await received(name)).toEqual([])
            }
            expect(warn.mock.calls.filter(([text]) => String(text).includes('Attempted to send message'))).toHaveLength(5)
        })

        it('delivers between plugins that name each other without a wildcard', async () => {
            await startIpcPlugins([
                { name: 'left', allowedIPC: ['left', 'right'] },
                { name: 'right', allowedIPC: ['right', 'left'] },
            ])

            expect(await post('left', 'right', 'inbox', 'hello')).toBe('resolved')

            expect(await waitFor(() => received('right'), (value) => value.length === 1)).toEqual([
                { message: 'hello', sender: 'left', channel: 'inbox' },
            ])
        })

        it('keeps listeners apart when plugin and channel names concatenate to the same string', async () => {
            await startIpcPlugins([
                { name: 'sender', allowedIPC: ['*'] },
                { name: 'a', allowedIPC: ['*'], channel: 'bc' },
                { name: 'ab', allowedIPC: ['*'], channel: 'c' },
            ])

            expect(await post('sender', 'a', 'bc', 'for a')).toBe('resolved')
            expect(await post('sender', 'ab', 'c', 'for ab')).toBe('resolved')

            expect(await waitFor(() => received('a'), (value) => value.length === 1)).toEqual([
                { message: 'for a', sender: 'sender', channel: 'bc' },
            ])
            expect(await waitFor(() => received('ab'), (value) => value.length === 1)).toEqual([
                { message: 'for ab', sender: 'sender', channel: 'c' },
            ])
        })
    })

    it('reaches the private conversation context read from the guest with the db decision', async () => {
        const name = 'context-reader'
        const readConversationContext = vi.fn(async () => ({ revision: 7, characterId: 'char' }))
        vi.mocked(createProductionPluginDatabaseAccess).mockReturnValueOnce({ readConversationContext } as never)
        vi.mocked(linkPluginQueryAbortSignals).mockImplementation((...signals) => {
            const controller = new AbortController()
            for (const signal of signals) signal?.addEventListener('abort', () => controller.abort(signal.reason))
            return { signal: controller.signal, dispose() {} }
        })
        mocks.confirm.mockResolvedValueOnce(false)
        await startFixture(name, 'globalThis.rpcReady = Promise.resolve()')

        const result = await guest(name, `
            const context = await risuai.risunestReadConversationContext({
                characterId: 'char',
                conversationId: 'conv',
                include: { persona: true },
                messages: { limit: 2 },
            });
            return JSON.stringify(context);
        `)

        expect(JSON.parse(String(result))).toEqual({ revision: 7, characterId: 'char' })
        expect(mocks.confirm).toHaveBeenCalledTimes(1)
        expect(readConversationContext).toHaveBeenCalledWith(
            expect.objectContaining({
                target: { characterId: 'char', conversationId: 'conv' },
                messages: { window: { limit: 2 }, extraFields: [] },
            }),
            { allowPrivate: false, signal: expect.any(AbortSignal) },
        )
    })

    it('does not log RPC request or response payloads when DEV is false', async () => {
        vi.stubEnv('DEV', false)
        installFixtureDatabase()
        mocks.permissionValues.clear()
        mocks.confirm.mockResolvedValue(true)
        const log = vi.spyOn(console, 'log').mockImplementation(() => undefined)
        await executePluginV3({ name: pluginName, script: fixtureScript } as any)
        await guestEvaluation
        await guest(pluginName, 'await globalThis.rpcReady; return true')

        expect(log.mock.calls.map(([first]) => first)).not.toContain('Original request:')
        expect(log.mock.calls.map(([first]) => first)).not.toContain('Original response:')
    })
})
