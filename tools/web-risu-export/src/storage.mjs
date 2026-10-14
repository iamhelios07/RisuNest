import * as oauth from 'openid-client';
import { ExportError, check, decoder, encoder, fail, fromHex, keyHex, sourceOf, wait } from './bytes.mjs';
import { references, scanDatabase } from './scan.mjs';

const DB_KEY = 'database/database.bin';
// The self-hosted Node server keeps saves on the server, not in this browser.
export function checkPlatform() {
    if (globalThis.__NODE__) fail('nodeServer');
}
export class IdbStore {
    constructor(db, store) { this.db = db; this.store = store; }
    static async open(name, store = 'keyvaluepairs', factory = indexedDB) {
        return new Promise((resolve, reject) => {
            let missing = false, blocked = false;
            const request = factory.open(name);
            request.onupgradeneeded = () => { missing = true; request.transaction.abort(); };
            request.onerror = () => missing ? resolve(null) : reject(request.error);
            request.onblocked = () => { blocked = true; reject(new ExportError('storageBlocked')); };
            request.onsuccess = () => {
                const db = request.result;
                if (blocked) { db.close(); return; }
                if (!db.objectStoreNames.contains(store)) { db.close(); resolve(null); }
                else { db.onversionchange = () => db.close(); resolve(new IdbStore(db, store)); }
            };
        });
    }
    get(key) {
        return new Promise((resolve, reject) => {
            check(this.signal);
            const tx = this.db.transaction(this.store, 'readonly');
            const abort = () => { try { tx.abort(); } catch {} };
            const cleanup = () => this.signal?.removeEventListener('abort', abort);
            this.signal?.addEventListener('abort', abort, { once: true });
            const request = tx.objectStore(this.store).get(key);
            request.onsuccess = () => { cleanup(); resolve(request.result); };
            request.onerror = () => { cleanup(); reject(this.signal?.aborted ? this.signal.reason : request.error); };
            tx.onabort = () => { cleanup(); reject(this.signal?.aborted ? this.signal.reason : tx.error); };
        });
    }
    async keys(signal) {
        const all = [];
        let last;
        do {
            check(signal);
            const batch = await new Promise((resolve, reject) => {
                const tx = this.db.transaction(this.store, 'readonly');
                const request = tx.objectStore(this.store).getAllKeys(last == null ? null : IDBKeyRange.lowerBound(last, true), 512);
                request.onsuccess = () => resolve(request.result);
                request.onerror = () => reject(request.error);
            });
            if (!batch.length) break;
            if (batch.some(key => typeof key !== 'string')) fail('invalidName');
            all.push(...batch);
            last = batch.at(-1);
            if (batch.length < 512) break;
        } while (true);
        return all;
    }
    close() { this.db.close(); }
}
export class AccountReader {
    constructor({ storage = localStorage, fetcher = (...args) => globalThis.fetch(...args), getKeys, signal, legacyToken = '' }) {
        this.storage = storage; this.fetcher = fetcher; this.getKeys = getKeys; this.signal = signal;
        this.legacyToken = legacyToken; this.accessToken = ''; this.expires = 0; this.refreshing = null;
        this.hub = storage.getItem('hub') === 'nightly' ? 'https://nightly.sv.risuai.xyz' : 'https://sv.risuai.xyz';
    }
    async refresh() {
        if (this.refreshing) return this.refreshing;
        this.refreshing = (async () => {
            check(this.signal);
            try {
                const encoded = this.storage.getItem('oauthData.json');
                if (!encoded) fail('login');
                const data = JSON.parse(decoder.decode(Uint8Array.from(atob(encoded), c => c.charCodeAt(0))));
                if (!data.refresh_token || !data.client_id) fail('login');
                const keys = await this.getKeys();
                if (!keys?.privateKey || keys.privateKey.extractable) fail('login');
                const config = await oauth.discovery(new URL('https://account.sionyw.com/'), data.client_id, data.client_secret, undefined, {
                    [oauth.customFetch]: (url, options) => this.fetcher(url, { ...options, signal: this.signal })
                });
                const tokens = await oauth.refreshTokenGrant(config, data.refresh_token, undefined, { DPoP: oauth.getDPoPHandle(config, keys) });
                this.accessToken = tokens.access_token;
                this.expires = Date.now() + (tokens.expires_in ?? 300) * 1000;
                if (tokens.refresh_token && tokens.refresh_token !== data.refresh_token) {
                    // Token rotation must survive the export; never overwrite a concurrent login.
                    if (this.storage.getItem('oauthData.json') !== encoded) fail('changed');
                    data.refresh_token = tokens.refresh_token;
                    const bytes = encoder.encode(JSON.stringify(data));
                    this.storage.setItem('oauthData.json', btoa(Array.from(bytes, b => String.fromCharCode(b)).join('')));
                }
            } catch (error) {
                check(this.signal);
                if (error?.code === 'changed') throw error;
                fail('login');
            }
        })();
        try { await this.refreshing; } finally { this.refreshing = null; }
    }
    async request(path, headers = {}) {
        const legacy = this.storage.getItem('ignoreRisuAuth') === 'true' ? '' : this.legacyToken;
        for (let attempt = 0; attempt < 4; attempt++) {
            check(this.signal);
            if (!legacy && this.expires - Date.now() < 60000) await this.refresh();
            let response;
            const timeout = new AbortController();
            const timer = setTimeout(() => timeout.abort(), 60000);
            try {
                response = await this.fetcher(this.hub + path, {
                    method: 'GET', cache: 'no-store', signal: AbortSignal.any([this.signal, timeout.signal].filter(Boolean)),
                    headers: { ...headers, ...(legacy ? { 'x-risu-auth': legacy } : { Authorization: 'Bearer ' + this.accessToken }) }
                });
            } catch {
                check(this.signal);
                if (attempt === 3) fail('network');
                await wait(500 * 2 ** attempt, this.signal); continue;
            } finally { clearTimeout(timer); }
            if ([401, 403].includes(response.status) || response.headers.get('WWW-Authenticate')?.includes('invalid_token')) {
                await response.body?.cancel().catch(() => {});
                if (legacy || attempt === 3) fail('login');
                this.expires = 0; continue;
            }
            if ([408, 429, 500, 502, 503, 504].includes(response.status)) {
                const retry = response.headers.get('retry-after');
                const seconds = Number(retry);
                const delay = retry && Number.isFinite(seconds) ? seconds * 1000
                    : retry && Number.isFinite(Date.parse(retry)) ? Date.parse(retry) - Date.now() : 500 * 2 ** attempt;
                await response.body?.cancel().catch(() => {});
                if (attempt === 3) fail('network');
                await wait(Math.max(0, Math.min(delay, 30000)), this.signal); continue;
            }
            return response;
        }
        fail('network');
    }
    async read(key) {
        return this.request('/api/account/read/' + keyHex(key) + (key === DB_KEY ? '|' + crypto.randomUUID() : ''), {
            'x-risu-key': key, 'x-risu-save-date': '0'
        });
    }
}
// A body that stalls or ends short of its declared size is `interrupted` so the entry can restart.
export function responseSource(response, signal, idle = 60000) {
    if (!response.ok || response.status === 204 || !response.body) {
        void response.body?.cancel().catch(() => {});
        fail(response.status === 204 || response.status === 404 || response.ok ? 'missing' : 'network');
    }
    const raw = response.headers.get('x-body-size') ?? (response.headers.get('content-encoding') ? null : response.headers.get('content-length'));
    const size = raw !== null && /^\d+$/.test(raw) ? Number(raw) : null;
    let used = false;
    return {
        size,
        stream() {
            if (used) fail('invalidData');
            used = true;
            const reader = response.body.getReader();
            let received = 0;
            const stop = (controller, error) => {
                void reader.cancel().catch(() => {});
                controller.error(signal?.aborted ? signal.reason : error instanceof ExportError ? error : new ExportError('interrupted'));
            };
            return new ReadableStream({
                async pull(controller) {
                    let timer, item;
                    try {
                        item = await Promise.race([reader.read(), new Promise((_, reject) => {
                            timer = setTimeout(() => reject(new ExportError('interrupted')), idle);
                        })]);
                    } catch (error) { stop(controller, error); return; }
                    finally { clearTimeout(timer); }
                    if (item.done) {
                        if (size != null && received !== size) stop(controller, new ExportError('interrupted'));
                        else controller.close();
                        return;
                    }
                    received += item.value.length;
                    if (size != null && received > size) { stop(controller, new ExportError('changed')); return; }
                    controller.enqueue(item.value);
                },
                cancel(reason) { return reader.cancel(reason); }
            }, { highWaterMark: 0 });
        },
        async cancel() { if (!used) await response.body.cancel().catch(() => {}); }
    };
}
async function fileAt(root, name) {
    if (!root) return null;
    try { return await (await root.getFileHandle(name)).getFile(); }
    catch (error) { if (error.name === 'NotFoundError') return null; throw error; }
}
export class BrowserSource {
    static async open(signal, onMode = () => {}) {
        checkPlatform();
        const source = new BrowserSource();
        source.abortController = new AbortController();
        source.onAbort = () => source.abortController.abort(signal.reason);
        signal?.addEventListener('abort', source.onAbort, { once: true });
        source.parentSignal = signal;
        if (signal?.aborted) source.onAbort();
        signal = source.abortController.signal;
        source.signal = signal; source.stores = [];
        source.flags = ['accountst', 'opfs_flag!', 'hub', 'ignoreRisuAuth'].map(key => localStorage.getItem(key));
        try {
            const open = async (name, store) => {
                const db = await IdbStore.open(name, store);
                if (db) { db.signal = signal; source.stores.push(db); }
                return db;
            };
            source.local = await open('risuai');
            source.inlays = await open('inlay', 'inlay');
            try { source.root = await navigator.storage?.getDirectory(); }
            catch (error) {
                if (error.name !== 'SecurityError' && error.name !== 'NotSupportedError') throw error;
                source.opfsError = error;
            }
            if (source.flags[0] === 'able') {
                source.mode = 'account';
                source.cache = await open('localforage');
                source.dbCache = await open('risuaiAccountCached');
                let legacyToken = '';
                try { legacyToken = JSON.parse(localStorage.getItem('fallbackRisuToken') ?? '{}').token ?? ''; } catch {}
                if (!legacyToken && !localStorage.getItem('oauthData.json') && source.dbCache) {
                    const cached = await source.dbCache.get(DB_KEY);
                    if (cached) {
                        const refs = references();
                        await scanDatabase(sourceOf(cached).stream(), refs, signal);
                        legacyToken = refs.token;
                    }
                }
                source.account = new AccountReader({
                    signal, legacyToken,
                    getKeys: async () => {
                        const db = await open('DPoPDB', 'DPoPStore');
                        return db?.get('dpop');
                    }
                });
            } else {
                source.mode = 'indexeddb';
                if (navigator.storage?.getDirectory && globalThis.FileSystemFileHandle?.prototype?.createWritable && source.flags[1] === 'able') {
                    if (!source.root) throw source.opfsError ?? new ExportError('denied');
                    const keys = await source.local?.keys(signal) ?? [];
                    if (!keys.includes(DB_KEY) || await source.local.get('migrated')) source.mode = 'opfs';
                    else if (!await source.local.get('denied_opfs')) fail('migration');
                }
            }
            onMode(source.mode);
            return source;
        } catch (error) { source.close(); throw error; }
    }
    async inventory() {
        check(this.signal);
        const assets = [], cold = [];
        if (this.root) {
            for await (const [name, handle] of this.root.entries()) {
                if (handle.kind !== 'file') continue;
                if (/^coldstorage_[0-9a-f-]+\.json$/i.test(name)) cold.push(name.slice(12, -5));
                if (this.mode === 'opfs') {
                    const key = fromHex(name);
                    if (key?.startsWith('assets/')) assets.push(key);
                }
            }
        }
        if (this.mode !== 'opfs') {
            const db = this.mode === 'account' ? this.cache : this.local;
            assets.push(...(await db?.keys(this.signal) ?? []).filter(key => key.startsWith('assets/')));
        }
        const inlays = await this.inlays?.keys(this.signal) ?? [];
        return { assets: assets.sort(), cold: cold.sort(), inlays: inlays.sort() };
    }
    async database() {
        if (this.mode === 'account') {
            const response = await this.account.read(DB_KEY);
            if (response.status === 303) {
                const match = await response.json();
                const date = await this.dbCache?.get(DB_KEY + '__date');
                const cached = date === '0' || date === 0 ? await this.dbCache?.get(DB_KEY) : null;
                if (match?.match && cached) return sourceOf(cached);
                fail('changed');
            }
            try { return responseSource(response, this.signal); }
            catch (error) { if (error.code === 'missing') fail('missingDatabase'); throw error; }
        }
        const value = this.mode === 'opfs' ? await fileAt(this.root, keyHex(DB_KEY)) : await this.local?.get(DB_KEY);
        if (!value) fail('missingDatabase');
        return sourceOf(value);
    }
    async asset(key) {
        check(this.signal);
        const local = this.mode === 'opfs' ? await fileAt(this.root, keyHex(key))
            : await (this.mode === 'account' ? this.cache : this.local)?.get(key);
        if (local != null) return sourceOf(local);
        if (this.mode !== 'account') fail('missing');
        return responseSource(await this.account.read(key), this.signal);
    }
    async cold(key) {
        if (this.mode === 'account') {
            const response = await this.account.request('/hub/account/coldstorage', { 'x-risu-key': key });
            if (response.status === 200) return responseSource(response, this.signal);
            await response.body?.cancel().catch(() => {});
            if (![404, 204].includes(response.status)) fail('network');
        }
        const file = await fileAt(this.root, 'coldstorage_' + key + '.json');
        if (!file) fail('missing');
        return sourceOf(file);
    }
    // A removed inlay is skipped here and reported as a change by verifyInventory.
    async inlay(key) { return this.inlays?.get(key); }
    async verifyInventory(expected) {
        const flags = ['accountst', 'opfs_flag!', 'hub', 'ignoreRisuAuth'].map(key => localStorage.getItem(key));
        if (JSON.stringify(flags) !== JSON.stringify(this.flags)) fail('changed');
        const actual = await this.inventory();
        if (JSON.stringify(actual) !== JSON.stringify(expected)) fail('changed');
    }
    cancel() { this.abortController.abort(); }
    close() {
        this.parentSignal?.removeEventListener('abort', this.onAbort);
        for (const db of this.stores ?? []) db.close();
    }
}
