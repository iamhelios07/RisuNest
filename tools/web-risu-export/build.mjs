import { build } from 'esbuild';
import { readFile, mkdir, writeFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const directory = path.dirname(fileURLToPath(import.meta.url));
const pkg = JSON.parse(await readFile(path.join(directory, 'package.json'), 'utf8'));
const result = await build({
    absWorkingDir: directory, entryPoints: ['src/main.mjs'], bundle: true,
    format: 'iife', target: ['chrome110', 'firefox115'], platform: 'browser',
    outfile: 'risunest-export.js',
    write: false, minify: false, legalComments: 'external', charset: 'utf8'
});
const header = `// ==UserScript==
// @name         RisuNest Web RisuAI Export
// @namespace    https://github.com/rsyumi/RisuNest
// @version      ${pkg.version}
// @description  Export saved RisuAI data and all local inlays to a RisuNest backup.
// @description:ko 웹리스 데이터와 모든 로컬 인레이를 RisuNest 백업으로 내보냅니다.
// @match        https://risuai.xyz/*
// @match        https://www.risuai.xyz/*
// @homepageURL  https://github.com/rsyumi/RisuNest
// @supportURL   https://github.com/rsyumi/RisuNest/issues
// @downloadURL  none
// @grant        none
// @noframes
// @run-at       document-idle
// @license      GPL-3.0
// ==/UserScript==
`;
const bundle = result.outputFiles.find(file => file.path.endsWith('.js')).text;
const extractedNotices = result.outputFiles.find(file => file.path.endsWith('.LEGAL.txt'))?.text.trim();
const notices = await readFile(path.join(directory, 'THIRD_PARTY_LICENSES.md'), 'utf8');
const licenseFile = notices.trimEnd() + (extractedNotices ? '\n\n## Bundled source notices\n\n```text\n' + extractedNotices + '\n```' : '') + '\n';
const licenseReference = '// Third-party licenses: THIRD_PARTY_LICENSES.md\n';
const output = path.join(directory, 'dist');
await mkdir(output, { recursive: true });
await writeFile(path.join(output, 'risunest-export.user.js'), header + licenseReference + bundle);
await writeFile(path.join(output, 'risunest-export.console.js'), licenseReference + bundle);
await writeFile(path.join(output, 'THIRD_PARTY_LICENSES.md'), licenseFile);
console.log('Built userscript, console bundle, and third-party license file.');
