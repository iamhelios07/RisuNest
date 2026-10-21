import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { readFile, mkdir, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { chromium, firefox } from 'playwright';
import { archive, coldId, database, readBlocks } from './fixtures.mjs';
import { gzipSync } from 'node:zlib';

const directory = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const bundle = await readFile(path.join(directory, 'dist/risunest-export.console.js'));
const server = createServer((request, response) => {
    if (request.url === '/bundle.js') { response.setHeader('content-type', 'text/javascript'); response.end(bundle); }
    else { response.setHeader('content-type', 'text/html; charset=utf-8'); response.end('<!doctype html><meta charset="utf-8"><title>Synthetic export test</title><body></body>'); }
});
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
const origin = 'http://127.0.0.1:' + server.address().port;
const scratch = path.join(directory, '.tmp/browser');
// Cold-stored plugin storage, read from OPFS locally and from the account server in account mode.
const [pluginIndex, pluginLarge] = ['50000000-0000-4000-8000-000000000001', '50000000-0000-4000-8000-000000000002'];
const pluginCold = new Map([
    [pluginIndex, gzipSync(JSON.stringify({ type: 'risu-plugin-storage-index', version: 1, trash: [], entries: [{ k: 'small', s: 'false' }, { k: 'large', l: pluginLarge }] }))],
    [pluginLarge, gzipSync(JSON.stringify({ text: '큰 값'.repeat(4096) }))]
]);
await mkdir(scratch, { recursive: true });
let browser;
const results = [];
try {
    const engine = process.argv.includes('--firefox') || process.env.EXPORT_BROWSER === 'firefox' ? firefox : chromium;
    browser = await engine.launch({ headless: true });
    for (const mode of engine === firefox ? ['firefox-fallback'] : ['indexeddb', 'opfs', 'account', 'firefox-fallback']) {
        const context = await browser.newContext({ locale: 'ko-KR', acceptDownloads: true });
        await context.route('**/*', route => route.request().url().startsWith(origin + '/') ? route.continue() : route.abort());
        const page = await context.newPage();
        const errors = [];
        page.on('pageerror', error => errors.push(error.message));
        await page.goto(origin);
        const db = database({
            character: {
                image: 'assets/synthetic.webp', ...(mode === 'account' ? { coldstorage: coldId } : {}),
                ...(mode === 'indexeddb' ? { emotionImages: [['sad', 'assets/missing.png']] } : {})
            },
            ...(mode === 'account' ? { root: { characterOrder: [{ img: 'assets/folder.png' }] }, presets: [{ image: 'assets/preset.png' }] } : {}),
            ...(mode !== 'indexeddb' ? { pluginStorage: { _coldpluginIndex: pluginIndex, inline: 1 } } : {})
        });
        if (mode === 'account') {
            await context.route('https://sv.risuai.xyz/**', async route => {
                const request = route.request(), path = new URL(request.url()).pathname;
                const headers = {
                    'access-control-allow-origin': origin,
                    'access-control-allow-methods': 'GET, OPTIONS',
                    'access-control-allow-headers': 'x-risu-auth,x-risu-key,x-risu-save-date,authorization'
                };
                if (request.method() === 'OPTIONS') return route.fulfill({ status: 204, headers });
                assert.equal(request.headers()['x-risu-auth'], 'synthetic-account-token');
                let body;
                if (path.startsWith('/api/account/read/')) {
                    const key = request.headers()['x-risu-key'];
                    if (key === 'database/database.bin') body = db;
                    else if (key === 'assets/synthetic.webp') body = Buffer.alloc(8 * 1024 * 1024, 37);
                    else if (key === 'assets/cold.png') body = Buffer.from([5, 6]);
                    else if (key === 'assets/folder.png') body = Buffer.from([8, 9]);
                    else if (key === 'assets/preset.png') body = Buffer.from([10, 11]);
                    else throw new Error('Unexpected synthetic asset request');
                } else if (path === '/hub/account/coldstorage') {
                    const key = request.headers()['x-risu-key'];
                    if (key === coldId) body = gzipSync(Buffer.from(JSON.stringify({ character: { chaId: 'character-1', image: 'assets/cold.png' } })));
                    else body = pluginCold.get(key);
                    assert(body, 'Unexpected synthetic cold storage request');
                } else return route.abort();
                return route.fulfill({ status: 200, headers, body });
            });
        }
        await page.evaluate(async ({ db, stale, mode, pluginCold }) => {
            async function store(name, storeName, pairs) {
                const database = await new Promise((resolve, reject) => {
                    const request = indexedDB.open(name);
                    request.onupgradeneeded = () => request.result.createObjectStore(storeName);
                    request.onsuccess = () => resolve(request.result); request.onerror = () => reject(request.error);
                });
                await new Promise((resolve, reject) => {
                    const tx = database.transaction(storeName, 'readwrite');
                    for (const [key, value] of pairs) tx.objectStore(storeName).put(value, key);
                    tx.oncomplete = resolve; tx.onerror = () => reject(tx.error);
                });
                database.close();
            }
            const root = await navigator.storage.getDirectory();
            const bytes = Uint8Array.from(db);
            if (mode !== 'account') for (const [key, value] of pluginCold) {
                const writable = await (await root.getFileHandle('coldstorage_' + key + '.json', { create: true })).createWritable();
                await writable.write(Uint8Array.from(value)); await writable.close();
            }
            const media = new Blob([new Uint8Array(8 * 1024 * 1024).fill(37)], { type: 'image/webp' });
            if (mode === 'opfs') {
                localStorage.setItem('opfs_flag!', 'able');
                const enc = key => Array.from(new TextEncoder().encode(key), b => b.toString(16).padStart(2, '0')).join('');
                for (const [key, value] of [['database/database.bin', bytes], ['assets/synthetic.webp', media]]) {
                    const writable = await (await root.getFileHandle(enc(key), { create: true })).createWritable();
                    await writable.write(value); await writable.close();
                }
                // The migration marker makes the deliberately stale IDB copy non-authoritative.
                await store('risuai', 'keyvaluepairs', [['database/database.bin', Uint8Array.from(stale)], ['migrated', true]]);
            } else {
                await store('risuai', 'keyvaluepairs', [['database/database.bin', bytes], ['assets/synthetic.webp', media]]);
            }
            await store('inlay', 'inlay', [
                ['orphan-한글', { name: 'image', type: 'image', ext: 'png', data: new Blob([Uint8Array.of(1, 2, 3)], { type: 'image/png' }), width: 1, height: 1 }],
                ['signature', { name: 'sig', type: 'signature', ext: 'json', data: '{"signatures":[]}' }]
            ]);
            if (mode === 'account') {
                localStorage.setItem('accountst', 'able');
                localStorage.setItem('fallbackRisuToken', JSON.stringify({ token: 'synthetic-account-token' }));
                await store('localforage', 'keyvaluepairs', [['assets/cached.png', new Blob([Uint8Array.of(7)])]]);
            }
            window.fixtureWriteDelay = 8;
            if (mode === 'firefox-fallback') Object.defineProperty(window, 'showSaveFilePicker', { value: undefined, configurable: true });
            else window.showSaveFilePicker = async () => ({
                createWritable: async () => {
                    const stream = await (await root.getFileHandle('synthetic-output.bin', { create: true })).createWritable();
                    return {
                        write: async data => { await new Promise(resolve => setTimeout(resolve, window.fixtureWriteDelay)); return stream.write(data); },
                        seek: position => stream.seek(position), truncate: size => stream.truncate(size),
                        close: () => stream.close(), abort: () => stream.abort()
                    };
                }
            });
            window.fixtureTicks = 0;
            setInterval(() => window.fixtureTicks++, 20);
        }, { db: [...db], stale: [...database({ root: { previousSave: true } })], mode, pluginCold: [...pluginCold].map(([key, value]) => [key, [...value]]) });
        await page.addScriptTag({ url: origin + '/bundle.js' });
        const panel = page.locator('#risunest-web-export');
        await panel.getByText('RisuNest 백업 내보내기', { exact: true }).waitFor();
        await panel.getByRole('combobox').selectOption('en');
        await panel.getByText('RisuNest backup export', { exact: true }).waitFor();
        await panel.getByRole('checkbox').check();
        await panel.getByRole('button', { name: 'Export backup', exact: true }).click();
        try {
            await panel.getByRole('status').filter({ hasText: mode === 'firefox-fallback' ? 'Backup file ready' : 'Backup saved' }).waitFor({ timeout: 30000 });
        } catch (error) {
            console.log(JSON.stringify({ mode, status: await panel.getByRole('status').textContent(), errors }));
            await page.screenshot({ path: path.join(scratch, 'failure.png') });
            throw error;
        }
        assert(await page.evaluate(() => window.fixtureTicks) > 2);
        if (mode === 'indexeddb') {
            const skipped = panel.locator('.skipped');
            await skipped.getByText('The following items were skipped. Everything else is included in the backup.', { exact: true }).waitFor();
            assert.deepEqual(await skipped.locator('dt').allTextContents(), ['Missing assets']);
            assert.deepEqual(await skipped.locator('dd').allTextContents(), ['1']);
        }
        await page.screenshot({ path: path.join(scratch, mode + '.png') });
        let output;
        if (mode === 'firefox-fallback') {
            const downloading = page.waitForEvent('download');
            await panel.getByRole('button', { name: 'Download backup', exact: true }).click();
            const download = await downloading;
            const target = path.join(scratch, 'fallback.bin');
            await download.saveAs(target);
            output = await readFile(target);
            page.once('dialog', dialog => dialog.accept());
            await panel.getByRole('button', { name: 'Remove temporary files', exact: true }).click();
            await panel.getByRole('status').filter({ hasText: 'Temporary files removed' }).waitFor();
            assert.equal(await page.evaluate(async () => {
                const root = await navigator.storage.getDirectory();
                return (await Array.fromAsync(root.keys())).includes('risunest-web-export-v1');
            }), false);
        } else {
            const downloading = page.waitForEvent('download');
            await page.evaluate(async () => {
                const file = await (await (await navigator.storage.getDirectory()).getFileHandle('synthetic-output.bin')).getFile();
                window.fixtureDownload = URL.createObjectURL(file);
                const link = document.createElement('a'); link.href = window.fixtureDownload; link.download = 'synthetic.bin'; link.click();
            });
            const download = await downloading;
            const target = path.join(scratch, mode + '.bin');
            await download.saveAs(target); output = await readFile(target);
            await page.evaluate(() => URL.revokeObjectURL(window.fixtureDownload));
        }
        const entries = archive(output);
        if (mode !== 'indexeddb') {
            const blocks = readBlocks(entries.get('database.risudat'));
            assert.deepEqual(blocks.slice(0, -1).map(b => b.bytes), readBlocks(db).filter(b => b.name !== 'pluginStorage').map(b => b.bytes));
            assert.deepEqual(JSON.parse(blocks.at(-1).body), { small: false, large: { text: '큰 값'.repeat(4096) }, inline: 1 });
        } else assert.deepEqual(entries.get('database.risudat'), db);
        assert.equal(entries.get('synthetic.webp').length, 8 * 1024 * 1024);
        assert(entries.get('synthetic.webp').every(byte => byte === 37));
        assert.equal(entries.size, mode === 'account' ? 9 : 4);
        if (mode === 'account') {
            assert.deepEqual(entries.get('cached.png'), Buffer.from([7]));
            assert.deepEqual(entries.get('cold.png'), Buffer.from([5, 6]));
            assert.deepEqual(entries.get('folder.png'), Buffer.from([8, 9]));
            assert.deepEqual(entries.get('preset.png'), Buffer.from([10, 11]));
            assert.equal(JSON.parse(entries.get('coldstorage_' + coldId + '.json')).character.chaId, 'character-1');
        }
        if (mode === 'indexeddb') {
            await page.evaluate(() => { window.fixtureWriteDelay = 80; });
            await panel.getByRole('button', { name: 'Export backup', exact: true }).click();
            await panel.getByRole('button', { name: 'Cancel', exact: true }).click();
            await panel.getByRole('status').filter({ hasText: 'Export cancelled' }).waitFor();
            await panel.getByRole('combobox').selectOption('ko');
            await panel.getByRole('status').filter({ hasText: '내보내기가 취소되었습니다' }).waitFor();
            await page.evaluate(() => { globalThis.__NODE__ = true; });
            await panel.getByRole('button', { name: '백업 내보내기', exact: true }).click();
            await panel.getByRole('status').filter({ hasText: '서버의 저장 폴더를 대신 백업해주세요' }).waitFor();
            assert(await panel.locator('.skipped').isHidden());
            await page.evaluate(() => { delete globalThis.__NODE__; });
        }
        if (mode === 'opfs') {
            await page.evaluate(() => {
                navigator.storage.getDirectory = async () => { throw new DOMException('Synthetic storage denial', 'SecurityError'); };
            });
            await panel.getByRole('button', { name: 'Export backup', exact: true }).click();
            await panel.getByRole('status').filter({ hasText: 'File or storage access was denied' }).waitFor();
        }
        assert.deepEqual(errors, []);
        results.push({ mode, archiveBytes: output.length, entries: entries.size, passed: true });
        await context.close();
    }
    await writeFile(path.join(scratch, (process.argv.includes('--firefox') ? 'firefox' : process.env.EXPORT_BROWSER ?? 'chromium') + '-result.json'), JSON.stringify(results, null, 2));
    console.log(JSON.stringify(results));
} finally {
    await browser?.close();
    await new Promise(resolve => server.close(resolve));
}
