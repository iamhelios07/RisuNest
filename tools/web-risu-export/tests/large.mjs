import assert from 'node:assert/strict';
import { performance } from 'node:perf_hooks';
import { writeFile, mkdir } from 'node:fs/promises';
import { exportBackup } from '../src/export.mjs';
import { sourceOf, streamOf } from '../src/bytes.mjs';
import { database, rawBlock } from './fixtures.mjs';

const gib = 1024 ** 3;
const record = Buffer.from(JSON.stringify({ role: 'user', data: 'Synthetic text '.repeat(4000), chatId: '00000000' }));
const databaseTarget = Number(process.env.EXPORT_DB_MIB ?? 1024) * 1024 * 1024;
const count = Math.floor(databaseTarget / (record.length + 1));
const start = Buffer.from('{"chaId":"character-1","type":"character","chats":[{"id":"chat-1","message":[');
const end = Buffer.from(']}]}');
const bodyLength = start.length + end.length + count * record.length + count - 1;
const baseline = database();
const lastBlock = rawBlock(2, 'character-1', Buffer.from(JSON.stringify({ chaId: 'character-1', type: 'character', name: 'Synthetic', chats: [] })));
const prefix = baseline.subarray(0, baseline.length - lastBlock.length);
const blockPrefix = Buffer.alloc(7 + 'character-1'.length);
blockPrefix[0] = 2; blockPrefix[2] = 'character-1'.length;
blockPrefix.write('character-1', 3); blockPrefix.writeUInt32LE(bodyLength, 3 + 'character-1'.length);
const db = {
    size: prefix.length + blockPrefix.length + bodyLength,
    stream: () => streamOf((async function* () {
        yield prefix; yield blockPrefix; yield start;
        const current = Buffer.from(record);
        for (let i = 0; i < count; i++) {
            if (i) yield Buffer.from(',');
            current.write(String(i).padStart(8, '0'), current.length - 10);
            yield current;
        }
        yield end;
    })())
};
const assetChunk = new Uint8Array(256 * 1024).fill(57);
const media = {
    size: gib,
    stream: () => streamOf((async function* () { for (let i = 0; i < gib / assetChunk.length; i++) yield assetChunk; })())
};
const names = Array.from({ length: 10 }, (_, i) => 'assets/synthetic-' + i + '.bin');
let position = 0, maximumPosition = 0, closed = false, writes = 0, maxWrite = 0;
const sink = {
    async write(bytes) { position += bytes.length; maximumPosition = Math.max(maximumPosition, position); maxWrite = Math.max(maxWrite, bytes.length); writes++; },
    async seek(offset) { position = offset; },
    async close() { closed = true; },
    async abort() {}
};
let peak = process.memoryUsage(), progressCount = 0, lastLog = 0;
const begin = performance.now();
const result = await exportBackup({
    source: {
        mode: 'opfs',
        inventory: async () => ({ assets: names, inlays: [], cold: [] }),
        database: async () => db, asset: async () => media, verifyInventory: async () => {}
    },
    sink,
    progress(state) {
        progressCount++;
        if (performance.now() - lastLog > 5000) {
            console.log(JSON.stringify({ stage: state.stage, bytes: state.bytes, readBytes: state.readBytes }));
            lastLog = performance.now();
        }
        const memory = process.memoryUsage();
        for (const key of Object.keys(peak)) peak[key] = Math.max(peak[key], memory[key]);
    }
});
assert(closed);
assert.equal(result.items, 11);
assert.equal(result.bytes, maximumPosition);
assert(result.bytes > 10 * gib);
assert(maxWrite <= 1024 * 1024);
assert(progressCount > 5);
const report = { databaseBytes: db.size, mediaBytes: 10 * gib, archiveBytes: result.bytes, seconds: (performance.now() - begin) / 1000, peak, maxWrite, writes, progressCount, output: 'counting sink, no disk I/O' };
await mkdir(new URL('../.tmp/', import.meta.url), { recursive: true });
await writeFile(new URL('../.tmp/large-result.json', import.meta.url), JSON.stringify(report, null, 2));
console.log(JSON.stringify(report));
