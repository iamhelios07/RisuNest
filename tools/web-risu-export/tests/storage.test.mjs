import test from 'node:test';
import assert from 'node:assert/strict';
import 'fake-indexeddb/auto';
import { AccountReader, IdbStore, responseSource } from '../src/storage.mjs';
import { keyHex } from '../src/bytes.mjs';

const memoryStorage = initial => {
    const values = new Map(Object.entries(initial));
    return { getItem: key => values.get(key) ?? null, setItem: (key, value) => values.set(key, value) };
};
async function seed(name, store, values) {
    const db = await new Promise((resolve, reject) => {
        const request = indexedDB.open(name);
        request.onupgradeneeded = () => request.result.createObjectStore(store);
        request.onsuccess = () => resolve(request.result);
        request.onerror = () => reject(request.error);
    });
    await new Promise((resolve, reject) => {
        const tx = db.transaction(store, 'readwrite');
        for (const [key, value] of values) tx.objectStore(store).put(value, key);
        tx.oncomplete = resolve; tx.onerror = () => reject(tx.error);
    });
    db.close();
}
test('read-only IndexedDB adapter enumerates batches and never creates a missing store', async () => {
    const name = 'synthetic-' + crypto.randomUUID();
    await seed(name, 'inlay', Array.from({ length: 1100 }, (_, i) => ['key-' + String(i).padStart(4, '0'), { data: new Blob(['synthetic']), name: 'test' }]));
    const store = await IdbStore.open(name, 'inlay');
    assert.equal((await store.keys()).length, 1100);
    assert.equal(await (await store.get('key-0001')).data.text(), 'synthetic');
    store.close();
    const absent = name + '-missing';
    assert.equal(await IdbStore.open(absent), null);
    assert(!(await indexedDB.databases()).some(db => db.name === absent));
    assert.equal(await IdbStore.open(name, 'missing'), null);
});
test('legacy account adapter sends the pinned protocol and preserves the response as a stream', async () => {
    const requests = [];
    const account = new AccountReader({
        storage: memoryStorage({}), legacyToken: 'synthetic-secret',
        fetcher: async (url, options) => { requests.push({ url, options }); return new Response(Uint8Array.of(8, 9), { headers: { 'x-body-size': '2' } }); }
    });
    const response = await account.read('assets/a.png');
    const source = responseSource(response);
    assert.equal(requests[0].url, 'https://sv.risuai.xyz/api/account/read/' + keyHex('assets/a.png'));
    assert.equal(requests[0].options.headers['x-risu-auth'], 'synthetic-secret');
    assert.equal(requests[0].options.headers['x-risu-save-date'], '0');
    assert.equal(source.size, 2);
    assert.deepEqual(new Uint8Array(await new Response(source.stream()).arrayBuffer()), Uint8Array.of(8, 9));
    assert.throws(() => source.stream());
});
test('OAuth refresh uses the browser key, preserves rotated refresh tokens and serves concurrent resources', async () => {
    const original = { refresh_token: 'synthetic-old', client_id: 'synthetic-client', client_secret: 'synthetic-secret' };
    const storage = memoryStorage({ 'oauthData.json': btoa(JSON.stringify(original)), ignoreRisuAuth: 'true' });
    const keyPair = await crypto.subtle.generateKey({ name: 'ECDSA', namedCurve: 'P-256' }, false, ['sign', 'verify']);
    const seen = [];
    let grants = 0;
    const account = new AccountReader({
        storage, getKeys: async () => keyPair,
        fetcher: async (input, options) => {
            const url = String(input);
            if (url.endsWith('/.well-known/openid-configuration')) return Response.json({
                issuer: 'https://account.sionyw.com/', token_endpoint: 'https://account.sionyw.com/token'
            });
            if (url === 'https://account.sionyw.com/token') {
                grants++;
                const headers = new Headers(options.headers);
                assert(headers.get('dpop'));
                const body = new URLSearchParams(options.body);
                assert.equal(body.get('refresh_token'), 'synthetic-old');
                return Response.json({ token_type: 'DPoP', access_token: 'synthetic-access', refresh_token: 'synthetic-new', expires_in: 3600 });
            }
            seen.push(new Headers(options.headers).get('authorization'));
            return new Response('synthetic payload');
        }
    });
    await Promise.all([account.read('assets/one.png'), account.read('assets/two.png')]);
    assert.deepEqual(seen, ['Bearer synthetic-access', 'Bearer synthetic-access']);
    assert.equal(grants, 1);
    assert.equal(JSON.parse(atob(storage.getItem('oauthData.json'))).refresh_token, 'synthetic-new');
    assert.equal(keyPair.privateKey.extractable, false);
});
test('429 retry is bounded and cancellation interrupts backoff', async () => {
    const controller = new AbortController();
    const account = new AccountReader({
        storage: memoryStorage({}), legacyToken: 'synthetic', signal: controller.signal,
        fetcher: async () => new Response('', { status: 429, headers: { 'retry-after': '30' } })
    });
    const request = account.read('assets/a.png');
    controller.abort();
    await assert.rejects(request);
    const failed = new AccountReader({
        storage: memoryStorage({}), legacyToken: 'synthetic',
        fetcher: async () => new Response('', { status: 429, headers: { 'retry-after': '0' } })
    });
    await assert.rejects(failed.read('assets/a.png'), error => error.code === 'network');
});
