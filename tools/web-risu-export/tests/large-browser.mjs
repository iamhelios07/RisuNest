import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { readFile, mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises';
import { performance } from 'node:perf_hooks';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import { chromium } from 'playwright';
import { build } from 'esbuild';
import { database, rawBlock } from './fixtures.mjs';

const root = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const scratch = path.join(root, '.tmp');
await mkdir(scratch, { recursive: true });
const profile = await mkdtemp(path.join(scratch, 'large-browser-'));
const bundle = await readFile(path.join(root, 'dist/risunest-export.console.js'));
const hasher = await build({
    absWorkingDir: root, stdin: { contents: "import {sha256} from '@noble/hashes/sha2.js'; globalThis.fixtureSha256 = sha256;", resolveDir: root },
    bundle: true, format: 'iife', write: false, platform: 'browser'
});
const server = createServer((request, response) => {
    response.setHeader('content-type', request.url === '/bundle.js' ? 'text/javascript' : 'text/html');
    response.end(request.url === '/bundle.js' ? bundle : '<!doctype html><meta charset="utf-8"><title>Synthetic large export</title>');
});
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
const origin = 'http://127.0.0.1:' + server.address().port;
let context, sampler, peakHeap = 0, peakBacking = 0, metrics;
try {
    context = await chromium.launchPersistentContext(profile, { headless: true, locale: 'en-US' });
    await context.route('**/*', route => route.request().url().startsWith(origin + '/') ? route.continue() : route.abort());
    const page = await context.newPage();
    await page.goto(origin);
    const baseline = database();
    const last = rawBlock(2, 'character-1', Buffer.from(JSON.stringify({ chaId: 'character-1', type: 'character', name: 'Synthetic', chats: [] })));
    const prefix = [...baseline.subarray(0, baseline.length - last.length)];
    const seedStarted = performance.now();
    const sizes = await page.evaluate(async ({ prefix, small, indexed }) => {
        const gib = 1024 ** 3, mib = 1024 ** 2;
        const dbTarget = small ? 16 * mib : gib;
        const mediaSize = small || indexed ? 16 * mib : gib;
        const encoder = new TextEncoder();
        const key = value => Array.from(encoder.encode(value), b => b.toString(16).padStart(2, '0')).join('');
        const root = await navigator.storage.getDirectory();
        localStorage.setItem('opfs_flag!', 'able');
        const record = encoder.encode(JSON.stringify({ role: 'user', data: 'Synthetic text '.repeat(4000), chatId: '00000000' }));
        const count = Math.floor(dbTarget / (record.length + 1));
        const start = encoder.encode('{"chaId":"character-1","type":"character","chats":[{"id":"chat-1","message":[');
        const end = encoder.encode(']}]}');
        const bodySize = start.length + end.length + count * record.length + count - 1;
        const header = new Uint8Array(18); header[0] = 2; header[2] = 11;
        header.set(encoder.encode('character-1'), 3); new DataView(header.buffer).setUint32(14, bodySize, true);
        const dbHandle = await root.getFileHandle(key('database/database.bin'), { create: true });
        const dbWriter = await dbHandle.createWritable();
        await dbWriter.write(Uint8Array.from(prefix)); await dbWriter.write(header); await dbWriter.write(start);
        // Seed in batches so setup time does not dominate file I/O measurements.
        let batch = [], bytes = 0;
        for (let i = 0; i < count; i++) {
            const value = record.slice(); value.set(encoder.encode(String(i).padStart(8, '0')), value.length - 10);
            if (i) batch.push(Uint8Array.of(44));
            batch.push(value); bytes += value.length + 1;
            if (bytes >= 1024 * 1024) { await dbWriter.write(new Blob(batch)); batch = []; bytes = 0; }
        }
        if (batch.length) await dbWriter.write(new Blob(batch));
        await dbWriter.write(end); await dbWriter.close();
        const chunk = new Uint8Array(mib).fill(57);
        for (let i = 0; i < 10; i++) {
            const writer = await (await root.getFileHandle(key('assets/synthetic-' + i + '.bin'), { create: true })).createWritable();
            for (let bytes = 0; bytes < mediaSize; bytes += chunk.length) await writer.write(chunk);
            await writer.close();
        }
        if (indexed) {
            const db = await new Promise((resolve, reject) => {
                const request = indexedDB.open('risuai');
                request.onupgradeneeded = () => request.result.createObjectStore('keyvaluepairs');
                request.onsuccess = () => resolve(request.result); request.onerror = () => reject(request.error);
            });
            const put = (name, value) => new Promise((resolve, reject) => {
                const tx = db.transaction('keyvaluepairs', 'readwrite');
                tx.objectStore('keyvaluepairs').put(value, name);
                tx.oncomplete = resolve; tx.onerror = () => reject(tx.error);
            });
            await put('database/database.bin', new Uint8Array(await (await dbHandle.getFile()).arrayBuffer()));
            for (let i = 0; i < 10; i++) await put('assets/synthetic-' + i + '.bin', await (await root.getFileHandle(key('assets/synthetic-' + i + '.bin'))).getFile());
            db.close();
            localStorage.removeItem('opfs_flag!');
        }
        window.showSaveFilePicker = async () => ({
            createWritable: async () => (await root.getFileHandle('synthetic-output.bin', { create: true })).createWritable()
        });
        window.fixtureDelays = []; window.fixtureLastTick = performance.now();
        window.fixtureTimer = setInterval(() => {
            const now = performance.now();
            window.fixtureDelays.push(now - window.fixtureLastTick);
            window.fixtureLastTick = now;
        }, 50);
        return { dbBytes: (await dbHandle.getFile()).size, mediaBytes: 10 * mediaSize, mediaSize };
    }, { prefix, small: process.argv.includes('--small'), indexed: process.argv.includes('--indexeddb') });
    console.log(JSON.stringify({ stage: 'seeded', ...sizes, seconds: (performance.now() - seedStarted) / 1000 }));
    metrics = await context.newCDPSession(page);
    sampler = setInterval(async () => {
        try {
            const memory = await metrics.send('Runtime.getHeapUsage');
            peakHeap = Math.max(peakHeap, memory.usedSize);
            peakBacking = Math.max(peakBacking, memory.backingStorageSize ?? 0);
        } catch {}
    }, 500);
    await page.addScriptTag({ url: origin + '/bundle.js' });
    const panel = page.locator('#risunest-web-export');
    await panel.getByRole('checkbox').check();
    const started = performance.now();
    await panel.getByRole('button', { name: 'Export backup', exact: true }).click();
    await panel.getByRole('status').filter({ hasText: 'Backup saved' }).waitFor({ timeout: 300000 });
    const exportSeconds = (performance.now() - started) / 1000;
    clearInterval(sampler); sampler = null;
    const timing = await page.evaluate(() => {
        clearInterval(window.fixtureTimer);
        const sorted = window.fixtureDelays.sort((a, b) => a - b);
        return { ticks: sorted.length, p99TickMs: sorted[Math.floor(sorted.length * .99)], maxTickMs: sorted.at(-1) };
    });
    await page.screenshot({ path: path.join(scratch, 'large-browser.png') });
    console.log(JSON.stringify({ stage: 'exported', exportSeconds, peakHeap, ...timing }));
    await page.addScriptTag({ content: hasher.outputFiles[0].text });
    const verified = await page.evaluate(async ({ mediaSize }) => {
        const root = await navigator.storage.getDirectory();
        const file = await (await root.getFileHandle('synthetic-output.bin')).getFile();
        const decoder = new TextDecoder(), encoder = new TextEncoder();
        const digest = async blob => {
            const hash = window.fixtureSha256.create();
            for await (const part of blob.stream()) hash.update(part);
            return Array.from(hash.digest(), b => b.toString(16).padStart(2, '0')).join('');
        };
        const expected = window.fixtureSha256.create();
        const chunk = new Uint8Array(1024 * 1024).fill(57);
        for (let size = 0; size < mediaSize; size += chunk.length) expected.update(chunk);
        const expectedMedia = Array.from(expected.digest(), b => b.toString(16).padStart(2, '0')).join('');
        const dbKey = Array.from(encoder.encode('database/database.bin'), b => b.toString(16).padStart(2, '0')).join('');
        const expectedDb = await digest(await (await root.getFileHandle(dbKey)).getFile());
        let offset = 0, entries = 0;
        async function number() {
            const bytes = await file.slice(offset, offset + 4).arrayBuffer(); offset += 4;
            return new DataView(bytes).getUint32(0, true);
        }
        while (offset < file.size) {
            const nameSize = await number();
            const name = decoder.decode(await file.slice(offset, offset + nameSize).arrayBuffer()); offset += nameSize;
            const size = await number();
            if (offset + size > file.size) throw new Error('Truncated output');
            const actual = await digest(file.slice(offset, offset + size));
            if (actual !== (name === 'database.risudat' ? expectedDb : expectedMedia)) throw new Error('Payload hash mismatch');
            offset += size; entries++;
        }
        return { bytes: file.size, entries, hashesMatch: true };
    }, sizes);
    assert.equal(verified.entries, 11);
    assert(verified.bytes > sizes.dbBytes + sizes.mediaBytes);
    assert(timing.ticks > 2);
    const report = { browser: context.browser().version(), ...sizes, ...verified, exportSeconds, peakJsHeapBytes: peakHeap, peakArrayBufferAndStringBytes: peakBacking, storage: process.argv.includes('--indexeddb') ? 'indexeddb' : 'opfs', ...timing, output: 'real OPFS files and writable stream, synthetic profile' };
    await writeFile(path.join(scratch, process.argv.includes('--indexeddb') ? 'large-indexeddb-result.json' : 'large-browser-result.json'), JSON.stringify(report, null, 2));
    console.log(JSON.stringify(report));
} finally {
    clearInterval(sampler);
    await context?.close();
    await new Promise(resolve => server.close(resolve));
    const resolved = path.resolve(profile);
    if (!resolved.startsWith(path.resolve(scratch) + path.sep) || !path.basename(resolved).startsWith('large-browser-')) throw new Error('Unsafe test profile cleanup');
    await rm(resolved, { recursive: true, force: true, maxRetries: 3 });
}
