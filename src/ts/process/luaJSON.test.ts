// @vitest-environment node

import { readFile } from 'node:fs/promises'
import { afterAll, beforeAll, expect, test } from 'vitest'
import { LuaFactory, type LuaEngine } from 'wasmoon'

let engine: LuaEngine
let decode: (text: string) => string
beforeAll(async () => {
    const factory = new LuaFactory()
    await factory.mountFile('json.lua', await readFile('public/lua/json.lua', 'utf8'))
    engine = await factory.createEngine()
    await engine.doString(`json=require('json')
        function decode(text) return json.encode(json.decode(text)) end
        function badControl(byte) return json.decode('"abc'..string.char(byte)..'"') end`)
    const roundtrip = engine.global.get('decode')
    decode = text => JSON.parse(roundtrip(text))
})
afterAll(() => { engine?.global.close() })

test('decodes plain spans, every escaped control byte, quotes, slashes and Unicode exactly', () => {
    const values = ['', 'plain '.repeat(20_000), '\\"/\b\f\n\r\t', '한글 日本語 😀 é',
        Array.from({ length: 32 }, (_, i) => String.fromCharCode(i)).join('')]
    for (let i = 0; i < 256; i++) values.push(`prefix${String.fromCharCode(i)}suffix`)
    for (const value of values) expect(decode(JSON.stringify(value))).toBe(value)
    expect(decode('"\\u0061\\ud55c\\ud83d\\ude00"')).toBe('a한😀')
})

test('reports the original bad byte and line/column after skipped spans', () => {
    for (let i = 0; i < 32; i++) {
        expect(() => engine.global.get('badControl')(i))
            .toThrow('control character in string at line 1 col 5')
    }
    for (const value of ['"abc', '"plain ']) {
        expect(() => decode(value)).toThrow('expected closing quote for string at line 1 col 1')
    }
    expect(() => decode('"abc\\q"')).toThrow("invalid escape char 'q' in string at line 1 col 5")
    expect(() => decode('"abc\\u12"')).toThrow('invalid unicode escape in string at line 1 col 5')
    expect(() => decode('[\n "abc\x01"]')).toThrow('control character in string at line 2 col 6')
    expect(() => decode('"done" true')).toThrow('trailing garbage at line 1 col 8')
})
