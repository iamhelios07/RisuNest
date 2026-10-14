import test from 'node:test';
import assert from 'node:assert/strict';
import { gzipSync } from 'node:zlib';
import { createHash } from 'node:crypto';
import { ArchiveWriter, ByteReader, ExportError, U32_MAX, encoder, keyHex, sourceOf, streamOf } from '../src/bytes.mjs';
import { COLD_HEADER, JsonScan, references, scanDatabase } from '../src/scan.mjs';
import { exportBackup, inlaySource } from '../src/export.mjs';
import { MemorySink, archive, block, coldId, database, fixtureSource, msgpack } from './fixtures.mjs';

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
test('missing referenced assets, changed database and failed writes never finalize a backup', async () => {
    const missing = fixtureSource({ db: database({ character: { image: 'assets/missing.png' } }) });
    const changed = fixtureSource();
    let reads = 0;
    changed.database = async () => sourceOf(database({ root: { name: 'version-' + reads++ } }));
    const missingFolder = fixtureSource({ db: database({ root: { characterOrder: [{ img: 'assets/missing-folder.png' }] } }) });
    const missingPreset = fixtureSource({ db: database({ presets: [{ image: 'assets/missing-preset.png' }] }) });
    for (const source of [missing, missingFolder, missingPreset, changed]) {
        const sink = new MemorySink();
        await assert.rejects(exportBackup({ source, sink }));
        assert(sink.aborted && !sink.closed);
    }
    const sink = new MemorySink();
    sink.write = async () => { throw new DOMException('Synthetic full disk', 'QuotaExceededError'); };
    await assert.rejects(exportBackup({ source: fixtureSource(), sink }));
    assert(sink.aborted && !sink.closed);
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
