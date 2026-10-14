import { sha256 } from '@noble/hashes/sha2.js';
import { ArchiveWriter, ByteReader, check, chunks, digest, encoder, fail, hex, keyHex, sourceOf, streamOf, u32, yielding } from './bytes.mjs';
import { JsonScan, UUID, references, scanDatabase } from './scan.mjs';

export function assetName(key) {
    if (!key.startsWith('assets/')) fail('invalidName');
    const name = key.slice(7);
    if (name === 'database.risudat' || name === 'encryption.risudat'
        || /^inlay_(?:[0-9a-f]{2})+\.risuinlay$/.test(name)
        || /^coldstorage_[0-9a-f-]{36}\.json$/i.test(name)
        || /^coldstorage\/[^/]+\.json$/.test(name)
        || /^(?:inlay|inlay_sidecar|inlay_info|inlay_meta|inlay_thumb)\//.test(name)) fail('invalidName');
    return name;
}
export function inlaySource(key, record) {
    if (typeof key !== 'string' || !key || key.startsWith('assets/')) fail('invalidInlay');
    if (!record || !['image', 'audio', 'video', 'signature'].includes(record.type)
        || typeof record.name !== 'string' || typeof record.ext !== 'string') fail('invalidInlay');
    let payload, mime;
    if (record.data instanceof Blob) {
        payload = sourceOf(record.data); mime = record.data.type;
    } else if (typeof record.data === 'string' && record.type === 'signature') {
        payload = sourceOf(encoder.encode(record.data)); mime = 'application/json';
    } else if (typeof record.data === 'string') {
        const comma = record.data.indexOf(',');
        const header = record.data.slice(0, comma);
        if (comma < 0 || !/^data:[^,;]*;base64$/i.test(header)) fail('invalidInlay');
        mime = header.slice(5, -7);
        const length = record.data.length - comma - 1;
        if (length % 4 !== 0 || !length) fail('invalidInlay');
        const size = length / 4 * 3 - (record.data.endsWith('==') ? 2 : record.data.endsWith('=') ? 1 : 0);
        payload = {
            size,
            stream: () => streamOf((async function* () {
                for (let offset = comma + 1; offset < record.data.length; offset += 256 * 1024) {
                    let binary;
                    try { binary = atob(record.data.slice(offset, offset + 256 * 1024)); }
                    catch { fail('invalidInlay'); }
                    yield Uint8Array.from(binary, c => c.charCodeAt(0));
                }
            })())
        };
    } else fail('invalidInlay');
    const fallbackMime = { jpg: 'image/jpeg', jpeg: 'image/jpeg', png: 'image/png', json: 'application/json' };
    mime ||= fallbackMime[record.ext] ?? (record.type === 'signature' ? 'application/json' : record.type + '/' + record.ext);
    const { data, type, ...extra } = record;
    const metadata = { ...extra, key, kind: 'inlay', inlayType: type, mime, size: payload.size };
    for (const dimension of ['width', 'height']) {
        if (metadata[dimension] != null && (!Number.isSafeInteger(metadata[dimension]) || metadata[dimension] < 0)) fail('invalidInlay');
    }
    let header;
    try { header = encoder.encode(JSON.stringify(metadata)); } catch { fail('invalidInlay'); }
    if (header.length > 1024 * 1024) fail('invalidInlay');
    return {
        name: 'inlay_' + keyHex(key) + '.risuinlay',
        size: 4 + header.length + payload.size,
        stream: () => streamOf((async function* () {
            yield u32(header.length); yield header;
            yield* chunks(payload.stream());
        })())
    };
}
export async function exportBackup({ source, sink, signal, progress = () => {} }) {
    const state = { stage: 'inventory', mode: source.mode, bytes: 0, readBytes: 0, items: 0, total: null, started: performance.now() };
    let last = 0;
    const report = (patch = {}, force = false) => {
        Object.assign(state, patch);
        if (force || performance.now() - last >= 100) { last = performance.now(); progress({ ...state }); }
    };
    const writer = new ArchiveWriter(sink, signal, n => { state.bytes += n; report(); });
    const breathe = yielding(signal);
    let pending = null;
    try {
        report({}, true);
        const inventory = await source.inventory();
        const refs = references();
        report({ stage: 'database' }, true);
        let database = await source.database();
        const hash = sha256.create();
        await writer.begin('database.risudat', database.size);
        await scanDatabase(database.stream(), refs, signal, async bytes => {
            hash.update(bytes); state.readBytes += bytes.length;
            await writer.append(bytes); await breathe();
        });
        await writer.end();
        database = null;
        const initialHash = hex(hash.digest());
        state.items++;
        const coldKeys = new Set([...inventory.cold, ...refs.cold]);
        for (const key of coldKeys) {
            check(signal);
            if (!UUID.test(key)) fail('invalidCold');
            report({ stage: 'cold', total: null }, true);
            const compressed = await source.cold(key);
            let coldRoot = false, coldField = false;
            const parser = new JsonScan((path, value) => {
                if (!path.length) coldRoot = Array.isArray(value) ? 'array' : value && typeof value === 'object' ? 'object' : false;
                if (path.length === 1 && ['character', 'message'].includes(path[0])) coldField = true;
                refs.visit(path, value);
            });
            let size = 0;
            await writer.begin('coldstorage_' + key + '.json');
            const decoded = compressed.stream().pipeThrough(new DecompressionStream('gzip'));
            for await (const bytes of chunks(decoded, signal)) {
                size += bytes.length;
                if (size > 1024 * 1024 * 1024) fail('recordTooLarge');
                parser.write(bytes); await writer.append(bytes); await breathe();
            }
            parser.end();
            if (!coldRoot || coldRoot === 'object' && !coldField) fail('invalidCold');
            await writer.end(); state.items++;
            for (const next of refs.cold) coldKeys.add(next);
            if (coldKeys.size > 199999) fail('tooMany');
        }
        const assets = [...new Set([...inventory.assets, ...refs.assets])].sort();
        report({ stage: 'assets', total: state.items + assets.length + inventory.inlays.length }, true);
        // One next request overlaps output without materializing response bodies.
        const prepare = key => source.asset(key).then(value => ({ value }), error => ({ error }));
        if (assets.length) pending = prepare(assets[0]);
        for (let i = 0; i < assets.length; i++) {
            check(signal);
            const item = await pending; pending = null;
            if (item.error) throw item.error;
            if (i + 1 < assets.length && source.mode === 'account') pending = prepare(assets[i + 1]);
            await writer.entry(assetName(assets[i]), item.value);
            state.items++; report(); await breathe();
            if (i + 1 < assets.length && !pending) pending = prepare(assets[i + 1]);
        }
        report({ stage: 'inlays' }, true);
        for (const key of inventory.inlays) {
            check(signal);
            const entry = inlaySource(key, await source.inlay(key));
            await writer.entry(entry.name, entry);
            state.items++; report(); await breathe();
        }
        report({ stage: 'verify' }, true);
        await source.verifyInventory(inventory);
        const finalHash = await digest(await source.database(), signal, n => { state.readBytes += n; report(); });
        if (initialHash !== finalHash) fail('changed');
        await source.verifyInventory(inventory);
        report({ stage: 'finishing' }, true);
        await writer.finish();
        report({ stage: 'complete' }, true);
        return { bytes: writer.position, items: state.items };
    } catch (error) {
        source.cancel?.();
        await writer.abort();
        throw error;
    } finally {
        if (pending) { const next = await pending; await next.value?.cancel?.(); }
        source.close?.();
    }
}
