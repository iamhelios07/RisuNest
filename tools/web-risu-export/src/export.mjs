import { sha256 } from '@noble/hashes/sha2.js';
import { ArchiveWriter, ExportError, check, chunks, digest, encoder, fail, hex, keyHex, sourceOf, streamOf, u32, wait, yielding } from './bytes.mjs';
import { JsonScan, references, scanDatabase } from './scan.mjs';

const MAX_COLD = 1024 * 1024 * 1024;
const BASE64 = /^[A-Za-z0-9+/]*={0,2}$/;
// A download that stalls or ends early restarts its entry, paced like the request retries.
const RETRIES = 3;
const UNREADABLE = new Set(['invalidData', 'truncated', 'recordTooLarge', 'invalidCold']);
const unreadable = error => error instanceof ExportError && UNREADABLE.has(error.code);
async function* gunzip(source, signal) {
    // DecompressionStream reports corrupt gzip as a TypeError.
    try { yield* chunks(source.stream().pipeThrough(new DecompressionStream('gzip')), signal); }
    catch (error) { if (error instanceof TypeError) fail('invalidCold'); throw error; }
}

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
    if (!record || typeof record !== 'object' || typeof record.name !== 'string' || typeof record.ext !== 'string') fail('invalidInlay');
    // Inlays saved before upstream added `type` are images, kept as a data URI or a migrated Blob.
    const type = record.type ?? (record.data instanceof Blob || /^data:image\//i.test(record.data) ? 'image' : null);
    if (!['image', 'audio', 'video', 'signature'].includes(type)) fail('invalidInlay');
    let payload, mime;
    if (record.data instanceof Blob) {
        payload = sourceOf(record.data); mime = record.data.type;
    } else if (typeof record.data === 'string' && type === 'signature') {
        payload = sourceOf(encoder.encode(record.data)); mime = 'application/json';
    } else if (typeof record.data === 'string') {
        const comma = record.data.indexOf(',');
        const header = record.data.slice(0, comma);
        if (comma < 0 || !/^data:[^,;]*;base64$/i.test(header)) fail('invalidInlay');
        mime = header.slice(5, -7);
        const length = record.data.length - comma - 1;
        // Validate before writing so a bad payload can be skipped without a partial entry.
        if (length % 4 !== 0 || !length || !BASE64.test(record.data.slice(comma + 1))) fail('invalidInlay');
        const size = length / 4 * 3 - (record.data.endsWith('==') ? 2 : record.data.endsWith('=') ? 1 : 0);
        payload = {
            size,
            stream: () => streamOf((async function* () {
                for (let offset = comma + 1; offset < record.data.length; offset += 256 * 1024) {
                    const binary = atob(record.data.slice(offset, offset + 256 * 1024));
                    yield Uint8Array.from(binary, c => c.charCodeAt(0));
                }
            })())
        };
    } else fail('invalidInlay');
    const fallbackMime = { jpg: 'image/jpeg', jpeg: 'image/jpeg', png: 'image/png', json: 'application/json' };
    mime ||= fallbackMime[record.ext] ?? (type === 'signature' ? 'application/json' : type + '/' + record.ext);
    const { data, type: _, ...extra } = record;
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
    const skipped = { missingAssets: 0, missingCold: 0, unreadableCold: 0, pluginCold: 0, unsupportedInlays: 0 };
    const state = { stage: 'inventory', mode: source.mode, bytes: 0, readBytes: 0, items: 0, total: null, skipped, started: performance.now() };
    let last = 0;
    const report = (patch = {}, force = false) => {
        Object.assign(state, patch);
        if (force || performance.now() - last >= 100) { last = performance.now(); progress({ ...state, skipped: { ...skipped } }); }
    };
    const writer = new ArchiveWriter(sink, signal, n => { state.bytes += n; report(); });
    const breathe = yielding(signal);
    const retrying = async (first, reopen, write) => {
        let value = first;
        for (let retry = 0; ; retry++) {
            try { return await write(value); }
            catch (error) {
                check(signal);
                if (error?.code !== 'interrupted') throw error;
                if (retry === RETRIES) fail('network');
                await writer.rollback();
                await wait(500 * 2 ** retry, signal);
                value = await reopen();
            }
        }
    };
    // Only archived chats are restorable; anything else in cold storage is rolled back.
    const writeCold = async (key, compressed) => {
        const found = references();
        let root = null, chat = false, size = 0;
        const parser = new JsonScan((path, value) => {
            if (!path.length) root = Array.isArray(value) ? 'array' : value && typeof value === 'object' ? 'object' : 'scalar';
            if (path.length === 1 && ['character', 'message'].includes(path[0])) chat = true;
            found.visit(path, value);
        }, { cold: true });
        await writer.begin('coldstorage_' + key + '.json');
        for await (const bytes of gunzip(compressed, signal)) {
            size += bytes.length;
            if (size > MAX_COLD) fail('recordTooLarge');
            parser.write(bytes); await writer.append(bytes); await breathe();
        }
        parser.end();
        if (root !== 'array' && !(root === 'object' && chat)) fail('invalidCold');
        await writer.end();
        return found;
    };
    let pending = null;
    try {
        report({}, true);
        const inventory = await source.inventory();
        report({ stage: 'database' }, true);
        let refs, initialHash;
        await retrying(await source.database(), () => source.database(), async database => {
            refs = references();
            const hash = sha256.create();
            let read = 0;
            await writer.begin('database.risudat');
            try {
                await scanDatabase(database.stream(), refs, signal, async bytes => {
                    hash.update(bytes); read += bytes.length; state.readBytes += bytes.length;
                    await breathe();
                }, undefined, bytes => writer.append(bytes));
            } catch (error) { state.readBytes -= read; throw error; }
            if (database.size != null && read !== database.size) fail('changed');
            await writer.end();
            initialHash = hex(hash.digest());
        });
        state.items++;
        report({ stage: 'cold', total: null }, true);
        // Unreferenced cold files are unused upstream and never restored.
        const coldKeys = new Set(refs.cold);
        for (const key of coldKeys) {
            check(signal);
            let found;
            try {
                found = await retrying(await source.cold(key), () => source.cold(key), compressed => writeCold(key, compressed));
            } catch (error) {
                check(signal);
                const reason = error?.code === 'missing' ? 'missingCold' : unreadable(error) ? 'unreadableCold' : null;
                if (!reason) throw error;
                await writer.rollback();
                skipped[reason]++; report();
                continue;
            }
            for (const asset of found.assets) refs.assets.add(asset);
            for (const next of found.cold) coldKeys.add(next);
            state.items++; report();
        }
        // Upstream v3 plugin storage values live in cold storage; RisuNest cannot restore them yet.
        for (const key of refs.pluginCold) if (!coldKeys.has(key)) skipped.pluginCold++;
        const assets = [...new Set([...inventory.assets, ...refs.assets])].sort();
        report({ stage: 'assets', total: state.items + assets.length + inventory.inlays.length }, true);
        // One next request overlaps output without materializing response bodies.
        const prepare = key => source.asset(key).then(value => ({ value }), error => ({ error }));
        if (assets.length) pending = prepare(assets[0]);
        for (let i = 0; i < assets.length; i++) {
            check(signal);
            const key = assets[i], name = assetName(key);
            const item = await pending; pending = null;
            if (item.error && item.error.code !== 'missing') throw item.error;
            if (i + 1 < assets.length && source.mode === 'account') pending = prepare(assets[i + 1]);
            try {
                if (item.error) throw item.error;
                await retrying(item.value, () => source.asset(key), value => writer.entry(name, value));
                state.items++;
            } catch (error) {
                check(signal);
                if (error?.code !== 'missing') throw error;
                await writer.rollback();
                skipped.missingAssets++; state.total--;
            }
            report(); await breathe();
            if (i + 1 < assets.length && !pending) pending = prepare(assets[i + 1]);
        }
        report({ stage: 'inlays' }, true);
        for (const key of inventory.inlays) {
            check(signal);
            try {
                const entry = inlaySource(key, await source.inlay(key));
                await writer.entry(entry.name, entry);
                state.items++;
            } catch (error) {
                check(signal);
                if (error?.code !== 'invalidInlay') throw error;
                await writer.rollback();
                skipped.unsupportedInlays++; state.total--;
            }
            report(); await breathe();
        }
        report({ stage: 'verify' }, true);
        await source.verifyInventory(inventory);
        const finalHash = await retrying(await source.database(), () => source.database(),
            database => digest(database, signal, n => { state.readBytes += n; report(); }));
        if (initialHash !== finalHash) fail('changed');
        await source.verifyInventory(inventory);
        report({ stage: 'finishing' }, true);
        await writer.finish();
        report({ stage: 'complete' }, true);
        return { bytes: writer.position, items: state.items, skipped: { ...skipped } };
    } catch (error) {
        source.cancel?.();
        await writer.abort();
        throw error;
    } finally {
        if (pending) { const next = await pending; await next.value?.cancel?.(); }
        source.close?.();
    }
}
