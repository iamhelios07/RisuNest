import { gzipSync } from 'node:zlib';
import { ExportError, sourceOf } from '../src/bytes.mjs';

export function block(type, name, value, compressed = false) {
    const body = Buffer.from(JSON.stringify(value));
    return rawBlock(type, name, compressed ? gzipSync(body) : body, compressed);
}
export function rawBlock(type, name, body, compressed = false) {
    const label = Buffer.from(name);
    const header = Buffer.alloc(7 + label.length);
    header[0] = type; header[1] = Number(compressed); header[2] = label.length;
    label.copy(header, 3); header.writeUInt32LE(body.length, 3 + label.length);
    return Buffer.concat([header, body]);
}
// `pluginStorage` is the stored value, or a Buffer holding the whole encoded block.
export function database({ compressed = false, root = {}, character = {}, modules = [], presets = [], pluginStorage = {} } = {}) {
    const entries = [
        [4, 'preset', presets], [5, 'modules', modules], [9, 'plugins', []],
        [10, 'loadouts', []], [11, 'pluginStorage', pluginStorage], [0, 'config', { version: 1 }],
        [2, 'character-1', { chaId: 'character-1', type: 'character', name: 'Synthetic', chats: [], ...character }]
    ];
    entries.unshift([1, 'root', { formatversion: 7, __directory: entries.map(e => e[1]), ...root }]);
    return Buffer.concat([Buffer.from('RISUSAVE\0'), ...entries.map(e => Buffer.isBuffer(e[2]) ? e[2] : block(...e, compressed))]);
}
export function archive(bytes) {
    const buffer = Buffer.from(bytes);
    const result = new Map();
    let offset = 0;
    while (offset < buffer.length) {
        const nameSize = buffer.readUInt32LE(offset); offset += 4;
        const name = buffer.toString('utf8', offset, offset + nameSize); offset += nameSize;
        const size = buffer.readUInt32LE(offset); offset += 4;
        if (size > buffer.length - offset) throw new Error('Truncated fixture archive');
        if (result.has(name)) throw new Error('Duplicate fixture entry');
        result.set(name, buffer.subarray(offset, offset + size)); offset += size;
    }
    return result;
}
export class MemorySink {
    constructor() { this.bytes = Buffer.alloc(0); this.offset = 0; this.closed = false; this.aborted = false; }
    async write(bytes) {
        if (this.offset + bytes.length > this.bytes.length) {
            const next = Buffer.alloc(this.offset + bytes.length); this.bytes.copy(next); this.bytes = next;
        }
        this.bytes.set(bytes, this.offset); this.offset += bytes.length;
    }
    async seek(position) { this.offset = position; }
    async truncate(size) { this.bytes = this.bytes.subarray(0, size); this.offset = Math.min(this.offset, size); }
    async close() { this.closed = true; }
    async abort() { this.aborted = true; }
}
export function fixtureSource({ db = database(), assets = new Map(), inlays = new Map(), cold = new Map(), mode = 'indexeddb' } = {}) {
    return {
        mode,
        inventory: async () => ({ assets: [...assets.keys()], inlays: [...inlays.keys()], cold: [...cold.keys()] }),
        database: async () => sourceOf(db),
        asset: async key => { if (!assets.has(key)) throw new ExportError('missing'); return sourceOf(assets.get(key)); },
        inlay: async key => inlays.get(key),
        cold: async key => { if (!cold.has(key)) throw new ExportError('missing'); return sourceOf(cold.get(key)); },
        verifyInventory: async () => {},
        close() {}
    };
}
export function msgpack(value) {
    if (value == null) return Buffer.from([0xc0]);
    if (typeof value === 'boolean') return Buffer.from([value ? 0xc3 : 0xc2]);
    if (Number.isInteger(value) && value >= 0 && value < 128) return Buffer.from([value]);
    if (typeof value === 'string') {
        const bytes = Buffer.from(value);
        const header = Buffer.alloc(5); header[0] = 0xdb; header.writeUInt32BE(bytes.length, 1);
        return Buffer.concat([header, bytes]);
    }
    const entries = Array.isArray(value) ? value.map(msgpack) : Object.entries(value).flatMap(([k, v]) => [msgpack(k), msgpack(v)]);
    const count = Array.isArray(value) ? value.length : Object.keys(value).length;
    const header = Buffer.alloc(3); header[0] = Array.isArray(value) ? 0xdc : 0xde; header.writeUInt16BE(count, 1);
    return Buffer.concat([header, ...entries]);
}
export const coldId = '10000000-0000-4000-8000-000000000001';
