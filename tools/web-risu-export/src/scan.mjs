import { Tokenizer, TokenType as T } from '@streamparser/json';
import { ByteReader, ExportError, check, chunks, decoder, fail, streamOf, yielding } from './bytes.mjs';

export const COLD_HEADER = '\uEF01COLDSTORAGE\uEF01';
export const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
const MAX_STRING = 8 * 1024 * 1024;
const MAX_BLOCK = 1024 * 1024 * 1024;

// Token events avoid retaining enclosing characters, chats, or the root object.
export class JsonScan {
    constructor(visit) {
        this.visit = visit; this.stack = []; this.rootDone = false; this.pending = 0;
        this.decoder = new TextDecoder('utf-8', { fatal: true });
        this.tokenizer = new Tokenizer({ stringBufferSize: 64 * 1024 });
        this.tokenizer.onToken = token => { this.pending = 0; this.token(token); };
    }
    token({ token, value }) {
        const frame = this.stack.at(-1);
        if (frame?.state === 'colon') {
            if (token !== T.COLON) fail('invalidData');
            frame.state = 'value'; return;
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
        if (frame) frame.state = 'comma'; else this.rootDone = true;
        if (token === T.LEFT_BRACE || token === T.LEFT_BRACKET) {
            if (this.stack.length >= 256) fail('invalidData');
            const object = token === T.LEFT_BRACE;
            this.visit(path, object ? {} : []);
            this.stack.push({ path, object, state: object ? 'keyOrEnd' : 'valueOrEnd', index: 0 });
        } else if ([T.STRING, T.NUMBER, T.TRUE, T.FALSE, T.NULL].includes(token)) {
            if (typeof value === 'string' && value.length > MAX_STRING) fail('recordTooLarge');
            this.visit(path, value);
        } else fail('invalidData');
    }
    write(bytes) {
        this.pending += bytes.length;
        if (this.pending > MAX_STRING + 256 * 1024) fail('recordTooLarge');
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
        }
        catch (error) { if (error instanceof ExportError) throw error; fail('invalidData'); }
        if (!this.rootDone || this.stack.length) fail('truncated');
    }
}
export function references() {
    const assets = new Set(), cold = new Set();
    let token = '';
    function addCold(key) { if (!UUID.test(key)) fail('invalidCold'); cold.add(key); }
    return {
        assets, cold,
        get token() { return token; },
        visit(path, value) {
            if (typeof value !== 'string') return;
            const key = path.at(-1), parent = path.at(-2);
            if (key === 'token' && parent === 'account') token = value;
            if (key === 'coldstorage' && value) addCold(value);
            if (parent === 'coldStoragedChats' && value) addCold(value);
            if (key === 'data' && value.startsWith(COLD_HEADER)) addCold(value.slice(COLD_HEADER.length));
            if (!value.startsWith('assets/')) return;
            if (['customBackground', 'userIcon', 'image', 'icon', 'imgFile'].includes(key)
                || (key === 'uri' && path.includes('ccAssets'))
                || (parent === 'files' && path.includes('vits'))
                || (key === 1 && ['emotionImages', 'additionalAssets', 'assets'].includes(path.at(-3)))) {
                assets.add(value);
                if (assets.size + cold.size > 199999) fail('tooMany');
            }
        }
    };
}
export async function scanJson(stream, visit, signal, max = MAX_BLOCK) {
    const parser = new JsonScan(visit);
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
            if (length > MAX_STRING) fail('recordTooLarge');
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
export async function scanDatabase(stream, refs, signal, onChunk, maxDecodedBlockBytes = MAX_BLOCK) {
    const reader = new ByteReader(stream, signal, onChunk);
    try {
        const first = await reader.take(9);
        if (decoder.decode(first) !== 'RISUSAVE\0') {
            const tail = await reader.take(2);
            const prefix = [...first, ...tail];
            if (decoder.decode(Uint8Array.from(prefix.slice(0, 10))) !== '\0RISUSAVE\0' || ![7, 8, 9].includes(prefix[10])) fail('unsupported');
            const rest = streamOf(reader.rest());
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
        const names = new Set(), directory = new Set();
        let directoryFound = false;
        const expected = new Map([[0, 'config'], [1, 'root'], [4, 'preset'], [5, 'modules'], [9, 'plugins'], [10, 'loadouts'], [11, 'pluginStorage']]);
        while (await reader.available()) {
            const [type, compression, nameLength] = await reader.take(3);
            const name = decoder.decode(await reader.take(nameLength));
            const length = await reader.number(4, true);
            if (!name || names.has(name) || compression > 1) fail('invalidData');
            if (![0, 1, 2, 4, 5, 7, 9, 10, 11].includes(type)) fail('unsupported');
            if (expected.has(type) && expected.get(type) !== name) fail('invalidData');
            names.add(name);
            if (names.size > 200000) fail('tooMany');
            let characterId = null, rootType = null;
            let block = streamOf(reader.parts(length));
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
                if (![0, 4, 9, 10, 11].includes(type)) refs.visit(path, value);
            }, signal, maxDecodedBlockBytes);
            if ([4, 5, 9, 10].includes(type) ? rootType !== 'array' : rootType !== 'object') fail('invalidData');
            if (type === 2 || type === 7) {
                if (characterId !== name) fail('invalidData');
            }
        }
        for (const name of expected.values()) if (!names.has(name)) fail('invalidData');
        if (!directoryFound) fail('invalidData');
        for (const name of directory) if (!names.has(name)) fail('invalidData');
        for (const name of names) if (name !== 'root' && !directory.has(name)) fail('invalidData');
    } catch (error) {
        if (error instanceof ExportError || signal?.aborted) throw error;
        fail('invalidData');
    } finally { await reader.close(); }
}
