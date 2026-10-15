import { Tokenizer, TokenType as T } from '@streamparser/json';
import { ByteReader, ExportError, check, chunks, decoder, encoder, fail, streamOf, yielding } from './bytes.mjs';

export const COLD_HEADER = '\uEF01COLDSTORAGE\uEF01';
export const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
// Messages, field values and keys have independent bounds; streamed containers do not.
const MAX_STRING = 64 * 1024 * 1024;
const MAX_BLOCK = 1024 * 1024 * 1024;
// Longer MessagePack strings cannot hold a reference, so they are skipped undecoded.
const MAX_REFERENCE = 64 * 1024;
// An unset upstream pluginCustomStorage is an empty block; RisuNest 1.0.4 and older reject it.
const MAX_HELD_PLUGIN_STORAGE = 64 * 1024;
const EMPTY_PLUGIN_STORAGE = Uint8Array.of(11, 0, 13, ...encoder.encode('pluginStorage'), 2, 0, 0, 0, ...encoder.encode('{}'));

// Token events avoid retaining enclosing characters, chats, or the root object.
export class JsonScan {
    constructor(visit, { maxString = MAX_STRING, blank = false, cold = false } = {}) {
        this.visit = visit; this.stack = []; this.rootDone = false; this.pending = 0;
        this.maxString = maxString; this.blank = blank; this.cold = cold;
        this.receivedBytes = 0; this.previousScalar = null;
        this.decoder = new TextDecoder('utf-8', { fatal: true });
        this.tokenizer = new Tokenizer({ stringBufferSize: 64 * 1024 });
        this.tokenizer.onToken = token => { this.pending = 0; this.token(token); };
    }
    token({ token, value, offset }) {
        this.checkScalarEnd(offset);
        if (token === T.STRING && encoder.encode(JSON.stringify(value)).length > this.maxString) fail('recordTooLarge');
        if ([T.STRING, T.NUMBER, T.TRUE, T.FALSE, T.NULL].includes(token)) this.previousScalar = offset;
        for (const container of this.stack) {
            if (container.bounded && offset - container.start + 1 > this.maxString) fail('recordTooLarge');
        }
        const frame = this.stack.at(-1);
        if (frame?.state === 'colon') {
            if (token !== T.COLON) fail('invalidData');
            frame.state = 'value'; frame.valueStart = offset + 1; return;
        }
        if (frame?.state === 'comma') {
            if (token === T.COMMA) { frame.state = frame.object ? 'key' : 'value'; return; }
            if (token === (frame.object ? T.RIGHT_BRACE : T.RIGHT_BRACKET)) { this.stack.pop(); return; }
            fail('invalidData');
        }
        if (frame?.state === 'key' || frame?.state === 'keyOrEnd') {
            if (frame.state === 'keyOrEnd' && token === T.RIGHT_BRACE) { this.stack.pop(); return; }
            if (token !== T.STRING) fail('invalidData');
            frame.key = value; frame.state = 'colon'; return;
        }
        if (frame?.state === 'valueOrEnd' && token === T.RIGHT_BRACKET) { this.stack.pop(); return; }
        if (!frame && this.rootDone) fail('invalidData');
        const path = frame ? [...frame.path, frame.object ? frame.key : frame.index++] : [];
        const recordStart = frame?.object ? frame.valueStart : offset;
        if (this.previousScalar !== null) this.previousScalar = recordStart;
        if (frame) frame.state = 'comma'; else this.rootDone = true;
        if (token === T.LEFT_BRACE || token === T.LEFT_BRACKET) {
            if (this.stack.length >= 256) fail('invalidData');
            const object = token === T.LEFT_BRACE;
            this.visit(path, object ? {} : []);
            const relative = this.cold && path[0] === 'character' ? path.slice(1) : path;
            const streamed = relative.length === 0
                || relative.length === 1 && (relative[0] === 'chats' || this.cold && relative[0] === 'message')
                || relative[0] === 'chats' && typeof relative[1] === 'number'
                    && (relative.length === 2 || relative.length === 3 && relative[2] === 'message');
            this.stack.push({ path, object, state: object ? 'keyOrEnd' : 'valueOrEnd', index: 0, start: recordStart, bounded: !streamed });
        } else if ([T.STRING, T.NUMBER, T.TRUE, T.FALSE, T.NULL].includes(token)) {
            this.visit(path, value);
        } else fail('invalidData');
    }
    write(bytes) {
        this.receivedBytes += bytes.length;
        this.pending += bytes.length;
        if (this.pending > this.maxString + 256 * 1024) fail('recordTooLarge');
        try {
            const text = this.decoder.decode(bytes, { stream: true });
            if (text) this.tokenizer.write(text);
        }
        catch (error) { if (error instanceof ExportError) throw error; fail('invalidData'); }
    }
    end() {
        try {
            const tail = this.decoder.decode();
            if (tail) this.tokenizer.write(tail);
            this.tokenizer.end();
            this.checkScalarEnd(this.receivedBytes);
        }
        catch (error) { if (error instanceof ExportError) throw error; fail('invalidData'); }
        if (this.stack.length || !this.rootDone && !this.blank) fail('truncated');
    }
    checkScalarEnd(offset) {
        if (this.previousScalar !== null && offset - this.previousScalar > this.maxString) fail('recordTooLarge');
        this.previousScalar = null;
    }
}
export function references() {
    const assets = new Set(), cold = new Set(), pluginCold = new Set();
    let token = '';
    function addCold(key) { if (!UUID.test(key)) fail('invalidCold'); cold.add(key); }
    // Upstream v3 plugin storage keeps each value in cold storage under pluginCustomStorage._coldplugin.
    function visitPluginStorage(path, value) {
        if (path.length === 2 && path[0] === '_coldplugin' && typeof value === 'string' && value) pluginCold.add(value);
    }
    return {
        assets, cold, pluginCold, visitPluginStorage,
        get token() { return token; },
        visit(path, value) {
            if (typeof value !== 'string') return;
            const key = path.at(-1), parent = path.at(-2);
            if (path[0] === 'pluginCustomStorage') visitPluginStorage(path.slice(1), value);
            if (key === 'token' && parent === 'account') token = value;
            if (key === 'coldstorage' && value) addCold(value);
            if (parent === 'coldStoragedChats' && value) addCold(value);
            if (key === 'data' && value.startsWith(COLD_HEADER)) addCold(value.slice(COLD_HEADER.length));
            if (!value.startsWith('assets/')) return;
            if (['customBackground', 'userIcon', 'image', 'icon', 'imgFile'].includes(key)
                || (key === 'img' && path.at(-3) === 'characterOrder')
                || (key === 'uri' && path.includes('ccAssets'))
                || (parent === 'files' && path.includes('vits'))
                || (key === 1 && ['emotionImages', 'additionalAssets', 'assets'].includes(path.at(-3)))) {
                assets.add(value);
            }
        }
    };
}
export async function scanJson(stream, visit, signal, max = MAX_BLOCK, options) {
    const parser = new JsonScan(visit, options);
    const breathe = yielding(signal);
    let length = 0;
    for await (const bytes of chunks(stream, signal)) {
        length += bytes.length;
        if (length > max) fail('recordTooLarge');
        parser.write(bytes);
        await breathe();
    }
    parser.end();
    return length;
}
async function scanMessagePack(reader, visit, signal) {
    const breathe = yielding(signal);
    async function value(path, asKey = false) {
        check(signal);
        if (path.length > 256) fail('invalidData');
        const tag = await reader.number(1);
        let length, map = false, array = false;
        if ((tag & 0xe0) === 0xa0) length = tag & 31;
        else if (tag >= 0xd9 && tag <= 0xdb) length = await reader.number([1, 2, 4][tag - 0xd9]);
        if (length != null) {
            if (!asKey && length > MAX_REFERENCE) {
                for await (const _ of reader.parts(length)) await breathe();
                return;
            }
            const text = decoder.decode(await reader.take(length));
            if (!asKey) visit(path, text);
            return text;
        }
        if ((tag & 0xf0) === 0x80) { length = tag & 15; map = true; }
        else if ((tag & 0xf0) === 0x90) { length = tag & 15; array = true; }
        else if (tag === 0xdc || tag === 0xdd) { length = await reader.number(tag === 0xdc ? 2 : 4); array = true; }
        else if (tag === 0xde || tag === 0xdf) { length = await reader.number(tag === 0xde ? 2 : 4); map = true; }
        if (map || array) {
            if (asKey) fail('invalidData');
            visit(path, map ? {} : []);
            for (let i = 0; i < length; i++) {
                const key = map ? await value(path, true) : i;
                if (map && typeof key !== 'string') fail('invalidData');
                await value([...path, key]);
                if (i % 256 === 0) await breathe();
            }
            return;
        }
        if (asKey) fail('invalidData');
        if (tag <= 0x7f || tag >= 0xe0 || [0xc0, 0xc2, 0xc3].includes(tag)) {
            visit(path, tag === 0xc0 ? null : tag === 0xc2 ? false : tag === 0xc3 ? true : tag <= 0x7f ? tag : tag - 256);
            return;
        }
        const sizes = { 0xca: 4, 0xcb: 8, 0xcc: 1, 0xcd: 2, 0xce: 4, 0xcf: 8, 0xd0: 1, 0xd1: 2, 0xd2: 4, 0xd3: 8 };
        if (sizes[tag]) { await reader.take(sizes[tag]); return; }
        if (tag >= 0xc4 && tag <= 0xc6) length = await reader.number([1, 2, 4][tag - 0xc4]);
        else if (tag >= 0xc7 && tag <= 0xc9) { length = await reader.number([1, 2, 4][tag - 0xc7]); length++; }
        else if (tag >= 0xd4 && tag <= 0xd8) length = (1 << (tag - 0xd4)) + 1;
        else fail('unsupported');
        for await (const _ of reader.parts(length)) await breathe();
    }
    await value([]);
    if (await reader.available()) fail('invalidData');
}
async function* tap(parts, write) {
    for await (const part of parts) { await write(part); yield part; }
}
async function* hold(parts, held) {
    for await (const part of parts) { held.push(part.slice()); yield part; }
}
// Output failures keep their own error instead of reading as malformed input.
class OutputError { constructor(error) { this.error = error; } }
// `write` receives the database to store: the source bytes, except that an empty
// pluginStorage block becomes an empty object.
export async function scanDatabase(stream, refs, signal, onChunk, maxDecodedBlockBytes = MAX_BLOCK, write = null) {
    const reader = new ByteReader(stream, signal, onChunk);
    const output = write;
    write = output && (async bytes => { try { await output(bytes); } catch (error) { throw new OutputError(error); } });
    try {
        const first = await reader.take(9);
        if (decoder.decode(first) !== 'RISUSAVE\0') {
            const tail = await reader.take(2);
            const prefix = [...first, ...tail];
            if (decoder.decode(Uint8Array.from(prefix.slice(0, 10))) !== '\0RISUSAVE\0' || ![7, 8, 9].includes(prefix[10])) fail('unsupported');
            await write?.(Uint8Array.from(prefix));
            const rest = streamOf(write ? tap(reader.rest(), write) : reader.rest());
            const decoded = prefix[10] === 7 ? rest : rest.pipeThrough(new DecompressionStream('gzip'));
            const limited = new ByteReader(decoded, signal, bytes => {
                decodedBytes += bytes.length;
                if (decodedBytes > maxDecodedBlockBytes) fail('recordTooLarge');
            });
            let decodedBytes = 0;
            try { await scanMessagePack(limited, refs.visit, signal); }
            finally { await limited.close(); }
            return;
        }
        await write?.(first);
        const names = new Set(), directory = new Set();
        let directoryFound = false;
        const expected = new Map([[0, 'config'], [1, 'root'], [4, 'preset'], [5, 'modules'], [9, 'plugins'], [10, 'loadouts'], [11, 'pluginStorage']]);
        while (await reader.available()) {
            const head = await reader.take(3);
            const [type, compression, nameLength] = head;
            const label = await reader.take(nameLength);
            const name = decoder.decode(label);
            const size = await reader.take(4);
            const length = new DataView(size.buffer, size.byteOffset, 4).getUint32(0, true);
            if (!name || names.has(name) || compression > 1) fail('invalidData');
            if (![0, 1, 2, 4, 5, 7, 9, 10, 11].includes(type)) fail('unsupported');
            if (expected.has(type) && expected.get(type) !== name) fail('invalidData');
            names.add(name);
            const header = new Uint8Array(7 + nameLength);
            header.set(head); header.set(label, 3); header.set(size, 3 + nameLength);
            const held = write && type === 11 && length <= MAX_HELD_PLUGIN_STORAGE ? [] : null;
            if (write && !held) await write(header);
            let characterId = null, rootType = null;
            const parts = reader.parts(length);
            let block = streamOf(held ? hold(parts, held) : write ? tap(parts, write) : parts);
            if (compression) block = block.pipeThrough(new DecompressionStream('gzip'));
            await scanJson(block, (path, value) => {
                if (!path.length) rootType = Array.isArray(value) ? 'array' : value && typeof value === 'object' ? 'object' : 'scalar';
                if (path.length === 1 && path[0] === 'chaId') characterId = value;
                if (type === 1 && path[0] === '__directory') {
                    if (path.length === 1) {
                        if (!Array.isArray(value)) fail('invalidData');
                        directoryFound = true;
                    } else if (path.length === 2) {
                        if (typeof value !== 'string' || !value || value === 'root' || directory.has(value)) fail('invalidData');
                        directory.add(value);
                    } else fail('invalidData');
                }
                if (type === 11) refs.visitPluginStorage?.(path, value);
                else if (![0, 9, 10].includes(type)) refs.visit(path, value);
            }, signal, maxDecodedBlockBytes, { maxString: type === 2 || type === 7 ? MAX_STRING : maxDecodedBlockBytes, blank: type === 11 });
            if (type === 11 && rootType === null) {
                if (write && !held) fail('invalidData');
                if (held) await write(EMPTY_PLUGIN_STORAGE);
            } else {
                if ([4, 5, 9, 10].includes(type) ? rootType !== 'array' : rootType !== 'object') fail('invalidData');
                if (held) { await write(header); for (const part of held) await write(part); }
            }
            if (type === 2 || type === 7) {
                if (characterId !== name) fail('invalidData');
            }
        }
        for (const name of expected.values()) if (!names.has(name)) fail('invalidData');
        if (!directoryFound) fail('invalidData');
        for (const name of directory) if (!names.has(name)) fail('invalidData');
        for (const name of names) if (name !== 'root' && !directory.has(name)) fail('invalidData');
    } catch (error) {
        if (error instanceof OutputError) throw error.error;
        if (error instanceof ExportError || signal?.aborted) throw error;
        fail('invalidData');
    } finally { await reader.close(); }
}
