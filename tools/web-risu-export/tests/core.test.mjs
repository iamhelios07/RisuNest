import test from 'node:test';
import assert from 'node:assert/strict';
import { gzipSync } from 'node:zlib';
import { createHash } from 'node:crypto';
import { ArchiveWriter, ByteReader, ExportError, U32_MAX, encoder, keyHex, sourceOf, streamOf } from '../src/bytes.mjs';
import { COLD_HEADER, JsonScan, references, scanDatabase } from '../src/scan.mjs';
import { exportBackup, inlaySource } from '../src/export.mjs';
import { MemorySink, archive, block, coldId, database, fixtureSource, msgpack, rawBlock } from './fixtures.mjs';

function fragmented(bytes, width = 1) {
    return streamOf((async function* () {
        for (let i = 0; i < bytes.length; i += width) yield bytes.subarray(i, i + width);
    })());
}
test('incremental JSON visits leaves without retaining parents and rejects malformed/truncated JSON', () => {
    const values = [];
    const parser = new JsonScan((path, value) => values.push([path, value]));
    const bytes = encoder.encode('{"한글":[{"image":"assets/한글.png"}],"bool":true,"empty":[],"n":-1.25e2,"escape":"a\\\"b"}');
    for (const byte of bytes) parser.write(Uint8Array.of(byte));
    parser.end();
    assert(values.some(([path, value]) => path.join('/') === '한글/0/image' && value === 'assets/한글.png'));
    assert(values.some(([path, value]) => path[0] === 'n' && value === -125));
    assert.deepEqual(values[0][1], {});
    for (const value of ['{"a":}', '[1,]', '{"a":1,}', '{}{}', '{"a"', '["unfinished']) {
        const broken = new JsonScan(() => {});
        assert.throws(() => { broken.write(encoder.encode(value)); broken.end(); });
    }
});
test('current release block layout discovers assets and cold references across compressed chunk boundaries', async () => {
    for (const compressed of [false, true]) {
        const db = database({
            compressed, root: { customBackground: 'assets/background.webp', personas: [{ embeddedModule: { assets: [['x', 'assets/module.png']] } }], characterOrder: [{ img: 'assets/folder.png', imgFile: 'assets/folder-file.png' }] },
            presets: [{ image: 'assets/preset.png' }],
            character: {
                image: 'assets/icon.jpg', emotionImages: [['happy', 'assets/happy.png']],
                additionalAssets: [['file', 'assets/file.bin']], vits: { files: { voice: 'assets/voice.onnx' } },
                ccAssets: [{ uri: 'assets/card.webp' }], coldStoragedChats: [coldId],
                chats: [{ message: [{ data: COLD_HEADER + coldId }, { data: 'assets/not-an-asset.png' }] }]
            }
        });
        const refs = references(); let raw = 0;
        await scanDatabase(fragmented(db, 3), refs, undefined, bytes => { raw += bytes.length; });
        assert.equal(raw, db.length);
        assert.deepEqual([...refs.assets].sort(), ['background.webp', 'card.webp', 'file.bin', 'folder.png', 'folder-file.png', 'happy.png', 'icon.jpg', 'module.png', 'preset.png', 'voice.onnx'].map(v => 'assets/' + v).sort());
        assert.deepEqual([...refs.cold], [coldId]);
    }
});
test('raw and compressed legacy message-pack databases preserve reference discovery', async () => {
    const packed = msgpack({ characters: [{ image: 'assets/old.png', chats: [{ message: [{ data: COLD_HEADER + coldId }] }] }], characterOrder: [{ img: 'assets/folder.png' }], botPresets: [{ image: 'assets/preset.png' }] });
    for (const format of [7, 8, 9]) {
        const bytes = Buffer.concat([Buffer.from('\0RISUSAVE\0'), Buffer.from([format]), format === 7 ? packed : gzipSync(packed)]);
        const refs = references();
        await scanDatabase(fragmented(bytes, 2), refs);
        assert.deepEqual([...refs.assets].sort(), ['assets/folder.png', 'assets/old.png', 'assets/preset.png']);
        assert.deepEqual([...refs.cold], [coldId]);
        await assert.rejects(scanDatabase(fragmented(bytes, 2), references(), undefined, undefined, 32), error => error.code === 'recordTooLarge');
    }
});
test('rejects directory mismatches, unsupported blocks and truncation', async () => {
    const invalid = [
        database({ root: { __directory: [] } }),
        database({ root: { __directory: ['missing'] } }),
        Buffer.concat([database(), block(6, 'remote', {})]),
        database().subarray(0, -1)
    ];
    for (const bytes of invalid) await assert.rejects(scanDatabase(fragmented(bytes, 7), references()), ExportError);
});
test('backup contains original database bytes, every asset type, orphaned inlays and decoded cold JSON', async () => {
    const db = database({ character: { image: 'assets/icon.jpg', coldstorage: coldId }, root: { characterOrder: [{ img: 'assets/folder.png', imgFile: 'assets/folder-file.png' }] }, presets: [{ image: 'assets/preset.png' }] });
    const cold = Buffer.from(JSON.stringify({ character: { chaId: 'character-1', image: 'assets/archived.webp' } }));
    const media = Buffer.from([0, 1, 2, 253, 254, 255]);
    const inlays = new Map([
        ['한글-orphan', { name: 'photo', ext: 'png', type: 'image', width: 2, height: 1, extra: 'kept', data: new Blob([media], { type: 'image/png' }) }],
        ['audio', { name: 'sound', ext: 'wav', type: 'audio', data: new Blob([media]) }],
        ['video', { name: 'clip', ext: 'webm', type: 'video', data: 'data:video/webm;base64,' + media.toString('base64') }],
        ['signature', { name: 'sig', ext: 'json', type: 'signature', data: '{"signatures":[]}' }]
    ]);
    const source = fixtureSource({
        db, mode: 'account', assets: new Map(['icon.jpg', 'archived.webp', 'nested/model.onnx', 'folder.png', 'folder-file.png', 'preset.png'].map(name => ['assets/' + name, media])),
        cold: new Map([[coldId, gzipSync(cold)]]), inlays
    });
    // Only the unreferenced model is cached; all referenced images must be discovered.
    source.inventory = async () => ({ assets: ['assets/nested/model.onnx'], cold: [coldId], inlays: [...inlays.keys()] });
    const sink = new MemorySink(), progress = [];
    const result = await exportBackup({ source, sink, progress: item => progress.push(item) });
    const entries = archive(sink.bytes);
    assert.equal(result.items, 12);
    assert.equal(result.bytes, sink.bytes.length);
    assert(sink.closed && !sink.aborted);
    assert.deepEqual(entries.get('database.risudat'), db);
    assert.deepEqual(entries.get('coldstorage_' + coldId + '.json'), cold);
    for (const name of ['icon.jpg', 'archived.webp', 'nested/model.onnx', 'folder.png', 'folder-file.png', 'preset.png']) assert.deepEqual(entries.get(name), media);
    for (const [key, original] of inlays) {
        const bytes = entries.get('inlay_' + keyHex(key) + '.risuinlay');
        const size = bytes.readUInt32LE(0);
        const meta = JSON.parse(bytes.toString('utf8', 4, 4 + size));
        assert.equal(meta.key, key); assert.equal(meta.kind, 'inlay'); assert.equal(meta.inlayType, original.type);
        assert.equal(meta.data, undefined);
        assert.deepEqual(bytes.subarray(4 + size), original.type === 'signature' ? Buffer.from(original.data) : media);
        if (key === '한글-orphan') assert.equal(meta.extra, 'kept');
    }
    assert.deepEqual([...new Set(progress.map(p => p.stage))], ['inventory', 'database', 'cold', 'assets', 'inlays', 'verify', 'finishing', 'complete']);
    assert(progress.every((p, i) => !i || p.bytes >= progress[i - 1].bytes));
});
test('missing referenced assets and archived chats are skipped and reported in a finished backup', async () => {
    const absent = '20000000-0000-4000-8000-000000000009';
    const db = database({
        character: { image: 'assets/missing.png', coldstorage: absent },
        root: { characterOrder: [{ img: 'assets/missing-folder.png' }] }, presets: [{ image: 'assets/missing-preset.png' }]
    });
    const sink = new MemorySink(), progress = [];
    const result = await exportBackup({ source: fixtureSource({ db, assets: new Map([['assets/kept.png', Buffer.from([7])]]) }), sink, progress: item => progress.push(item) });
    assert(sink.closed && !sink.aborted);
    assert.deepEqual([...archive(sink.bytes).keys()].sort(), ['database.risudat', 'kept.png']);
    assert.deepEqual(result.skipped, { missingAssets: 3, missingCold: 1, unreadableCold: 0, pluginCold: 0, unsupportedInlays: 0 });
    assert.equal(result.items, 2);
    assert.equal(progress.at(-1).total, 2);
    assert.deepEqual(progress.at(-1).skipped, result.skipped);
});
test('changed databases and failed writes never finalize a backup', async () => {
    const changed = fixtureSource();
    let reads = 0;
    changed.database = async () => sourceOf(database({ root: { name: 'version-' + reads++ } }));
    const sink = new MemorySink();
    await assert.rejects(exportBackup({ source: changed, sink }), error => error.code === 'changed');
    assert(sink.aborted && !sink.closed);
    // The larger database outgrows the 1 MiB output buffer, so its write fails mid-scan.
    for (const db of [database(), database({ root: { padding: 'x'.repeat(2 * 1024 * 1024) } })]) {
        const full = new MemorySink();
        full.write = async () => { throw new DOMException('Synthetic full disk', 'QuotaExceededError'); };
        await assert.rejects(exportBackup({ source: fixtureSource({ db }), sink: full }), error => error.name === 'QuotaExceededError');
        assert(full.aborted && !full.closed);
    }
});
test('cancellation aborts output and stops consuming a large payload', async () => {
    const controller = new AbortController(), sink = new MemorySink(), source = fixtureSource();
    const count = 100;
    let pulled = 0;
    source.inventory = async () => ({ assets: ['assets/large.bin'], inlays: [], cold: [] });
    source.asset = async () => ({
        size: count * 256 * 1024,
        stream: () => streamOf((async function* () { for (let i = 0; i < count; i++) { pulled++; yield new Uint8Array(256 * 1024); } })())
    });
    const write = sink.write.bind(sink);
    sink.write = async bytes => { await write(bytes); controller.abort(); };
    await assert.rejects(exportBackup({ source, sink, signal: controller.signal }));
    assert(pulled < count); assert(sink.aborted && !sink.closed);
});
test('length-prefixed writer backpatches unknown sizes and rejects unsafe or duplicate entry names', async () => {
    const sink = new MemorySink(), writer = new ArchiveWriter(sink, undefined, () => {}, 13);
    await writer.begin('database.risudat');
    await writer.append(Uint8Array.of(1, 2, 3)); await writer.end(); await writer.finish();
    assert.deepEqual(archive(sink.bytes).get('database.risudat'), Buffer.from([1, 2, 3]));
    for (const name of ['../x', '/x', 'a\\b', 'x/./y', 'x//y', 'C:x']) {
        await assert.rejects(new ArchiveWriter(new MemorySink()).begin(name), ExportError);
    }
    await assert.rejects(writer.begin('database.risudat'), ExportError);
    await assert.rejects(new ArchiveWriter(new MemorySink()).begin('large', U32_MAX + 1), ExportError);
});
test('old base64 inlays decode in bounded chunks with an independent hash', async () => {
    const original = Buffer.alloc(1024 * 1024 + 2, 0xe7);
    const item = inlaySource('id', { name: 'image', ext: 'png', type: 'image', data: 'data:image/png;base64,' + original.toString('base64') });
    const reader = new ByteReader(item.stream());
    const headerSize = await reader.number(4, true);
    await reader.take(headerSize);
    const hash = createHash('sha256'); let max = 0;
    for await (const bytes of reader.rest()) { max = Math.max(max, bytes.length); hash.update(bytes); }
    await reader.close();
    assert.equal(hash.digest('hex'), createHash('sha256').update(original).digest('hex'));
    assert(max <= 256 * 1024);
});
test('an unset upstream plugin storage block is written as an empty object', async () => {
    const stored = rawBlock(11, 'pluginStorage', Buffer.from('{}'));
    for (const compressed of [false, true]) {
        for (const body of ['', ' \r\n\t']) {
            const encoded = compressed ? gzipSync(Buffer.from(body)) : Buffer.from(body);
            const sink = new MemorySink();
            await exportBackup({ source: fixtureSource({ db: database({ compressed, pluginStorage: rawBlock(11, 'pluginStorage', encoded, compressed) }) }), sink });
            assert.deepEqual(archive(sink.bytes).get('database.risudat'), database({ compressed, pluginStorage: stored }));
        }
    }
    assert.throws(() => new JsonScan(() => {}).end(), error => error.code === 'truncated');
});
test('record limits count UTF-8 and bound each field and message independently', async () => {
    const large = 'x'.repeat(8 * 1024 * 1024 + 1);
    await scanDatabase(sourceOf(database({ root: { large } })).stream(), references());
    await scanDatabase(sourceOf(database({ presets: [{ large }] })).stream(), references());
    await scanDatabase(sourceOf(database({ character: { large } })).stream(), references());
    const checkRecord = text => {
        const scan = new JsonScan(() => {}, { maxString: 64 });
        const bytes = encoder.encode(text);
        for (let offset = 0; offset < bytes.length; offset += 7) scan.write(bytes.subarray(offset, offset + 7));
        scan.end();
    };
    for (const value of [
        { field: 'x'.repeat(62) },
        { field: Array(10).fill('가') },
        { first: 'x'.repeat(40), second: 'y'.repeat(40) },
        { chats: [{ message: [{ data: 'x'.repeat(40) }, { data: 'y'.repeat(40) }] }] },
    ]) checkRecord(JSON.stringify(value));
    for (const text of [
        JSON.stringify({ field: '가'.repeat(22) }),
        JSON.stringify({ ['가'.repeat(22)]: 0 }),
        JSON.stringify({ field: Array(20).fill('aa') }),
        JSON.stringify({ field: Array(12).fill('가') }),
        '{"field":' + ' '.repeat(64) + '"x"}',
        JSON.stringify({ message: [{ first: 'x'.repeat(40), second: 'y'.repeat(40) }] }),
        '{"field":"' + '\\u0061'.repeat(11) + '"}',
    ]) assert.throws(() => checkRecord(text), error => error.code === 'recordTooLarge');
});
test('cold storage exports only restorable archived chats and reports what it skips', async () => {
    const [plugin, settings, corrupt, nested, scalar] = [1, 2, 3, 4, 5].map(n => '2000000' + n + '-0000-4000-8000-00000000000' + n);
    const db = database({
        pluginStorage: { _coldplugin: { synthetic: plugin } },
        character: { coldstorage: coldId, coldStoragedChats: [settings, corrupt, scalar], chats: [{ message: [{ data: COLD_HEADER + nested }] }] }
    });
    // The rejected payload is larger than the output buffer, so its rollback rewinds flushed bytes.
    const big = { image: 'assets/dropped.png', value: 'x'.repeat(2 * 1024 * 1024) };
    const cold = new Map([
        [coldId, gzipSync(JSON.stringify({ character: { chaId: 'character-1', image: 'assets/archived.png' } }))],
        [plugin, gzipSync('{"plugin":true}')],
        [settings, gzipSync(JSON.stringify(big))],
        [corrupt, Buffer.from('not gzip')],
        [nested, gzipSync(JSON.stringify([{ role: 'char', data: 'archived message' }]))],
        [scalar, gzipSync('"text"')]
    ]);
    const source = fixtureSource({ db, cold, assets: new Map([['assets/archived.png', Buffer.from([1])]]) });
    const read = source.cold, requested = [];
    source.cold = async key => { requested.push(key); return read(key); };
    const sink = new MemorySink();
    const result = await exportBackup({ source, sink });
    const entries = archive(sink.bytes);
    assert.deepEqual([...entries.keys()].sort(), ['archived.png', 'coldstorage_' + coldId + '.json', 'coldstorage_' + nested + '.json', 'database.risudat']);
    assert.deepEqual(JSON.parse(entries.get('coldstorage_' + nested + '.json')), [{ role: 'char', data: 'archived message' }]);
    assert.equal(result.bytes, sink.bytes.length);
    assert.deepEqual(result.skipped, { missingAssets: 0, missingCold: 0, unreadableCold: 3, pluginCold: 1, unsupportedInlays: 0 });
    assert(!requested.includes(plugin));
});
test('an output failure while writing an archived chat stops the export instead of skipping it', async () => {
    const db = database({ character: { coldstorage: coldId } });
    // The payload outgrows the output buffer, so the failing write happens inside the cold entry.
    const cold = new Map([[coldId, gzipSync(JSON.stringify({ character: { chaId: 'character-1', note: 'x'.repeat(2 * 1024 * 1024) } }))]]);
    const sink = new MemorySink(), progress = [];
    const write = sink.write.bind(sink);
    let writes = 0;
    // The first two writes are the database entry and its backpatched length.
    sink.write = async bytes => { if (++writes > 2) throw new TypeError('Synthetic closed output'); return write(bytes); };
    await assert.rejects(exportBackup({ source: fixtureSource({ db, cold }), sink, progress: item => progress.push(item) }), TypeError);
    assert.equal(progress.at(-1).stage, 'cold');
    assert(sink.aborted && !sink.closed);
});
test('legacy untyped image inlays are exported and unusable inlays are skipped', async () => {
    const media = Buffer.from([137, 80, 78, 71]);
    const inlays = new Map([
        ['legacy-uri', { name: 'old', ext: 'png', width: 3, height: 4, data: 'data:image/png;base64,' + media.toString('base64') }],
        ['legacy-blob', { name: 'migrated', ext: 'webp', data: new Blob([media], { type: 'image/webp' }) }],
        ['legacy-text', { name: 'note', ext: 'txt', data: 'data:text/plain;base64,' + media.toString('base64') }],
        ['bad-base64', { name: 'broken', ext: 'png', type: 'image', data: 'data:image/png;base64,ab!d' }],
        ['bad-size', { name: 'sized', ext: 'png', type: 'image', width: -1, data: new Blob([media]) }],
        ['removed', undefined]
    ]);
    const sink = new MemorySink();
    const result = await exportBackup({ source: fixtureSource({ inlays }), sink });
    const entries = archive(sink.bytes);
    for (const [key, mime] of [['legacy-uri', 'image/png'], ['legacy-blob', 'image/webp']]) {
        const bytes = entries.get('inlay_' + keyHex(key) + '.risuinlay');
        const size = bytes.readUInt32LE(0);
        const meta = JSON.parse(bytes.toString('utf8', 4, 4 + size));
        assert.equal(meta.inlayType, 'image'); assert.equal(meta.mime, mime);
        assert.deepEqual(bytes.subarray(4 + size), media);
    }
    assert.equal(entries.size, 3);
    assert.equal(result.skipped.unsupportedInlays, 4);
    assert(sink.closed && !sink.aborted);
});
test('interrupted account downloads restart their entry and give up after bounded retries', async () => {
    const media = Buffer.alloc(300 * 1024, 5);
    const db = database({ character: { image: 'assets/remote.png' } });
    const flaky = (bytes, failures) => {
        let opened = 0;
        const open = async () => {
            const attempt = opened++;
            return {
                size: bytes.length,
                stream: () => streamOf((async function* () {
                    yield bytes.subarray(0, 1000);
                    if (attempt < failures) throw new ExportError('interrupted');
                    yield bytes.subarray(1000);
                })())
            };
        };
        open.count = () => opened;
        return open;
    };
    const source = fixtureSource({ db, mode: 'account' });
    source.database = flaky(db, 1); source.asset = flaky(media, 1);
    const sink = new MemorySink(), progress = [];
    await exportBackup({ source, sink, progress: item => progress.push(item) });
    const entries = archive(sink.bytes);
    assert.deepEqual(entries.get('database.risudat'), db);
    assert.deepEqual(entries.get('remote.png'), media);
    assert.equal(source.database.count(), 3);
    assert.equal(source.asset.count(), 2);
    assert.equal(progress.at(-1).readBytes, 2 * db.length);
    const broken = fixtureSource({ db, mode: 'account' });
    broken.asset = flaky(media, Infinity);
    const failed = new MemorySink();
    await assert.rejects(exportBackup({ source: broken, sink: failed }), error => error.code === 'network');
    assert.equal(broken.asset.count(), 4);
    assert(failed.aborted && !failed.closed);
});
