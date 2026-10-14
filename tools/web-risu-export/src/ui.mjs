import { ExportError, check, fail } from './bytes.mjs';
import { BrowserSource, checkPlatform } from './storage.mjs';
import { exportBackup } from './export.mjs';

const text = {
    en: {
        title: 'RisuNest backup export', intro: 'Export this browser’s saved RisuAI data, assets, and all local inlays.',
        idle: 'Ready', confirmed: 'Saving has finished. I have stopped generation and edits in all tabs and devices.',
        start: 'Export backup', cancel: 'Cancel', close: 'Close', language: 'Language',
        inventory: 'Listing saved data', database: 'Reading database', cold: 'Exporting archived chats',
        assets: 'Exporting assets', inlays: 'Exporting inlays', verify: 'Checking for changes', finishing: 'Finishing file',
        complete: 'Backup saved', prepared: 'Backup file ready', cancelled: 'Export cancelled',
        written: 'Written', items: 'Items', elapsed: 'Elapsed', speed: 'Write speed', read: 'Database read', skipped: 'Skipped',
        skippedNote: 'The following items were skipped. Everything else is included in the backup.',
        missingAssets: 'Missing assets', missingCold: 'Missing archived chats', unreadableCold: 'Unreadable archived chats',
        pluginCold: 'Plugin storage data', unsupportedInlays: 'Unsupported inlays',
        local: 'Local', indexeddb: 'Local · IndexedDB', opfs: 'Local · OPFS', account: 'Account',
        download: 'Download backup', downloadStarted: 'Download started. Check your browser’s downloads before removing the temporary file.',
        firefox: 'This browser uses a temporary file before downloading. Additional storage roughly equal to the backup size is required. Keep this tab open until the download finishes.',
        cleanup: 'Remove temporary files', cleaned: 'Temporary files removed', cleanupConfirm: 'Remove export temporary files? Wait for any backup download to finish first.',
        active: 'An export is already running.', failed: 'Export failed. No complete backup was saved.',
        changed: 'Saved data changed during export. Stop edits and generation, wait for saving to finish, then try again.',
        missingDatabase: 'No saved RisuAI database was found on this page.',
        nodeServer: 'This RisuAI page saves data on its server, not in this browser. Back up the server’s save folder instead.',
        login: 'Account authentication failed. Sign in again in RisuAI, wait for saving to finish, then retry.',
        network: 'An account download failed. Check the connection and try again.',
        migration: 'A storage migration is incomplete. Reload RisuAI and wait for it to finish before exporting.',
        unsupported: 'This saved database format is not supported. Open and save it in the supported RisuAI release, then retry.',
        invalidData: 'Saved data is malformed or unsupported. Export was stopped.',
        invalidCold: 'An archived-chat reference is invalid. Export was stopped.',
        invalidName: 'An asset name cannot be preserved in this backup format. Export was stopped.',
        duplicate: 'Two entries would use the same backup name. Export was stopped.',
        truncated: 'A saved file ended unexpectedly. Export was stopped.',
        recordTooLarge: 'A database block or record exceeds the supported import limits.',
        entryTooLarge: 'A single backup entry exceeds 4 GiB minus 1 byte.',
        quota: 'Not enough browser storage for the temporary backup. Free space or use Chrome or Edge.',
        denied: 'File or storage access was denied. Check browser permissions and retry.',
        outputUnavailable: 'This browser does not provide a supported file output method.',
        storageBlocked: 'Browser storage is busy. Close other RisuAI tabs and retry.',
        persist: 'Request storage permission', permissionPending: 'Respond to the browser’s storage permission request.',
        permissionGranted: 'Storage permission granted. Try exporting again.', permissionDenied: 'Storage permission was not granted.'
    },
    ko: {
        title: 'RisuNest 백업 내보내기', intro: '이 브라우저에 저장된 웹리스 데이터와 에셋, 모든 로컬 인레이를 내보냅니다.',
        idle: '준비', confirmed: '저장이 완료되었으며, 모든 탭과 기기에서 생성과 편집을 중지했습니다.',
        start: '백업 내보내기', cancel: '취소', close: '닫기', language: '언어',
        inventory: '저장된 데이터 확인 중', database: 'DB 읽는 중', cold: '보관된 채팅 내보내는 중',
        assets: '에셋 내보내는 중', inlays: '인레이 내보내는 중', verify: '변경 사항 확인 중', finishing: '파일 저장 마무리 중',
        complete: '백업 저장 완료', prepared: '백업 파일 준비 완료', cancelled: '내보내기가 취소되었습니다',
        written: '기록한 용량', items: '항목', elapsed: '경과 시간', speed: '기록 속도', read: '읽은 DB 용량', skipped: '제외한 항목',
        skippedNote: '다음 항목은 백업에서 제외했습니다. 나머지 데이터는 모두 백업에 포함되었습니다.',
        missingAssets: '찾을 수 없는 에셋', missingCold: '찾을 수 없는 보관된 채팅', unreadableCold: '읽을 수 없는 보관된 채팅',
        pluginCold: '플러그인 저장 데이터', unsupportedInlays: '지원하지 않는 인레이',
        local: '로컬', indexeddb: '로컬 · IndexedDB', opfs: '로컬 · OPFS', account: '계정',
        download: '백업 다운로드', downloadStarted: '다운로드를 시작했습니다. 브라우저에서 다운로드 완료를 확인한 후 임시 파일을 삭제하세요.',
        firefox: '이 브라우저는 임시 파일을 만든 후 다운로드합니다. 백업 크기만큼의 추가 저장 공간이 필요합니다. 다운로드가 끝날 때까지 이 탭을 열어두세요.',
        cleanup: '임시 파일 삭제', cleaned: '임시 파일을 삭제했습니다', cleanupConfirm: '내보내기 임시 파일을 삭제하시겠습니까? 백업 다운로드가 끝난 후 삭제해주세요.',
        active: '내보내기가 진행 중입니다.', failed: '내보내기에 실패했습니다. 완성된 백업이 저장되지 않았습니다.',
        changed: '내보내기 중 저장된 데이터가 변경되었습니다. 편집과 생성을 중지하고 저장이 완료된 후 다시 시도해주세요.',
        missingDatabase: '이 페이지에서 저장된 웹리스 DB를 찾을 수 없습니다.',
        nodeServer: '이 웹리스 페이지는 데이터를 브라우저가 아닌 서버에 저장합니다. 서버의 저장 폴더를 대신 백업해주세요.',
        login: '계정 인증에 실패했습니다. 웹리스에 다시 로그인하고 저장이 완료된 후 다시 시도해주세요.',
        network: '계정 데이터 다운로드에 실패했습니다. 연결을 확인하고 다시 시도해주세요.',
        migration: '저장소 이동이 완료되지 않았습니다. 웹리스를 새로고침하고 이동이 끝난 후 내보내기해주세요.',
        unsupported: '지원하지 않는 DB 저장 형식입니다. 지원하는 웹리스 릴리즈에서 열고 저장한 후 다시 시도해주세요.',
        invalidData: '저장된 데이터가 잘못되었거나 지원하지 않는 형식입니다. 내보내기가 중지되었습니다.',
        invalidCold: '보관된 채팅 참조가 잘못되었습니다. 내보내기가 중지되었습니다.',
        invalidName: '백업 형식에서 보존할 수 없는 에셋 이름입니다. 내보내기가 중지되었습니다.',
        duplicate: '백업 파일 이름이 중복됩니다. 내보내기가 중지되었습니다.',
        truncated: '저장된 파일이 중간에 끊겼습니다. 내보내기가 중지되었습니다.',
        recordTooLarge: 'DB 블록 또는 레코드가 지원하는 복원 크기를 초과합니다.',
        entryTooLarge: '개별 백업 항목이 4 GiB에서 1바이트를 뺀 크기를 초과합니다.',
        quota: '임시 백업을 만들 브라우저 저장 공간이 부족합니다. 공간을 확보하거나 Chrome 또는 Edge를 사용해주세요.',
        denied: '파일 또는 저장소 접근이 거부되었습니다. 브라우저 권한을 확인하고 다시 시도해주세요.',
        outputUnavailable: '이 브라우저에서 지원하는 파일 저장 방법이 없습니다.',
        storageBlocked: '브라우저 저장소를 사용할 수 없습니다. 다른 웹리스 탭을 닫고 다시 시도해주세요.',
        persist: '저장 권한 요청', permissionPending: '브라우저의 저장 권한 요청을 확인해주세요.',
        permissionGranted: '저장 권한이 허용되었습니다. 내보내기를 다시 시도해주세요.', permissionDenied: '저장 권한이 허용되지 않았습니다.'
    }
};
const TEMP_DIR = 'risunest-web-export-v1';
const TEMP_NAME = /^archive-[0-9a-f-]{36}\.bin$/;
const formatBytes = value => {
    if (value < 1024) return value + ' B';
    const unit = Math.min(3, Math.floor(Math.log(value) / Math.log(1024)));
    return (value / 1024 ** unit).toFixed(1) + ' ' + ['B', 'KiB', 'MiB', 'GiB'][unit];
};
async function destination(picker, signal) {
    if (picker) {
        const handle = await picker;
        check(signal);
        return { sink: await handle.createWritable(), temporary: false };
    }
    if (!navigator.storage?.getDirectory) fail('outputUnavailable');
    check(signal);
    const directory = await (await navigator.storage.getDirectory()).getDirectoryHandle(TEMP_DIR, { create: true });
    const name = 'archive-' + crypto.randomUUID() + '.bin';
    const handle = await directory.getFileHandle(name, { create: true });
    try {
        const sink = await handle.createWritable();
        return { sink, temporary: true, file: () => handle.getFile(), remove: () => directory.removeEntry(name) };
    } catch (error) { await directory.removeEntry(name).catch(() => {}); throw error; }
}
async function clearTemporaryFiles() {
    if (!navigator.storage?.getDirectory) return;
    let dir;
    try { dir = await (await navigator.storage.getDirectory()).getDirectoryHandle(TEMP_DIR); }
    catch (error) { if (error.name === 'NotFoundError') return; throw error; }
    for await (const [name, handle] of dir.entries()) {
        if (handle.kind === 'file' && TEMP_NAME.test(name)) await dir.removeEntry(name);
    }
    // The folder belongs to this tool; keep it only if something else was placed in it.
    try { await (await navigator.storage.getDirectory()).removeEntry(TEMP_DIR); }
    catch (error) { if (!['NotFoundError', 'InvalidModificationError'].includes(error.name)) throw error; }
}
export function mount() {
    const existing = document.getElementById('risunest-web-export');
    if (existing) { existing.scrollIntoView(); return; }
    const host = document.createElement('div');
    host.id = 'risunest-web-export';
    const root = host.attachShadow({ mode: 'open' });
    root.innerHTML = `<style>
:host{all:initial;position:fixed;right:20px;bottom:20px;width:min(430px,calc(100vw - 24px));z-index:2147483647;color-scheme:dark;font:14px/1.5 system-ui,sans-serif;color:#eef2f6}
*{box-sizing:border-box}section{background:#18202b;border:1px solid #445163;border-radius:14px;padding:20px;box-shadow:0 12px 48px #0008;max-height:calc(100vh - 40px);overflow:auto}
header{display:flex;align-items:center;gap:10px}h1{font-size:18px;margin:0;flex:1}p{margin:12px 0;color:#c1cbd7}select,button{font:inherit;color:inherit;background:#283547;border:1px solid #53647b;border-radius:7px;padding:7px 10px}
button{cursor:pointer}button:disabled{opacity:.5;cursor:default}button.primary{background:#326ca4;border-color:#7daedb}button:hover:enabled{filter:brightness(1.15)}label{display:flex;gap:8px;margin:14px 0}input{margin-top:5px;flex:none}progress{width:100%;height:12px;accent-color:#7cb8ee}
dl{display:grid;grid-template-columns:1fr 1fr;gap:5px 12px;margin:12px 0}dt{color:#aebed0}dd{margin:0;text-align:right;font-variant-numeric:tabular-nums}.actions{display:flex;flex-wrap:wrap;gap:8px;margin-top:16px}.note{font-size:12px}.status{font-weight:600;color:#e4f0ff}.error{color:#ffb6b6}[hidden]{display:none!important}select{max-width:100px}
@media(max-width:480px){:host{right:12px;bottom:12px}section{padding:16px}}
</style><section role="dialog" aria-labelledby="export-title">
<header><h1 id="export-title"></h1><select aria-label="Language"><option value="ko">한국어</option><option value="en">English</option></select></header>
<p data-text="intro"></p><p class="note" data-text="firefox" hidden></p>
<label><input type="checkbox"><span data-text="confirmed"></span></label>
<p class="status" role="status" aria-live="polite"></p><progress hidden></progress>
<dl hidden><dt data-text="written"></dt><dd data-value="bytes"></dd><dt data-text="items"></dt><dd data-value="items"></dd><dt data-text="elapsed"></dt><dd data-value="elapsed"></dd><dt data-text="speed"></dt><dd data-value="speed"></dd><dt data-text="read"></dt><dd data-value="read"></dd><dt data-text="skipped" data-skipped hidden></dt><dd data-value="skipped" data-skipped hidden></dd></dl>
<div class="skipped" hidden><p class="note" data-text="skippedNote"></p><dl></dl></div>
<div class="actions"><button class="primary" data-action="start" data-text="start" disabled></button><button data-action="cancel" data-text="cancel" hidden></button><button class="primary" data-action="download" data-text="download" hidden></button><button data-action="persist" data-text="persist" hidden></button><button data-action="cleanup" data-text="cleanup" hidden></button><button data-action="close" data-text="close"></button></div>
</section>`;
    document.documentElement.append(host);
    const find = selector => root.querySelector(selector);
    const button = action => find('[data-action="' + action + '"]');
    let lang = navigator.language.toLowerCase().startsWith('ko') ? 'ko' : 'en';
    let busy = false, controller, state, status = 'idle', output, url, timer, endTime;
    const temporary = typeof window.showSaveFilePicker !== 'function';
    find('select').value = lang;
    find('[data-text="firefox"]').hidden = !temporary;
    button('cleanup').hidden = !temporary;
    const t = key => text[lang][key] ?? text[lang].failed;
    function render() {
        host.lang = lang;
        find('h1').textContent = t('title');
        find('select').setAttribute('aria-label', t('language'));
        for (const node of root.querySelectorAll('[data-text]')) node.textContent = t(node.dataset.text);
        const statusNode = find('.status');
        statusNode.textContent = t(status) + (busy && state?.mode ? ' · ' + t(state.mode) : '');
        if (state) {
            const seconds = Math.max(0.001, ((endTime ?? performance.now()) - state.started) / 1000);
            find('dl').hidden = false;
            find('[data-value="bytes"]').textContent = formatBytes(state.bytes);
            find('[data-value="read"]').textContent = formatBytes(state.readBytes);
            find('[data-value="items"]').textContent = state.items + (state.total == null ? '' : ' / ' + state.total);
            find('[data-value="elapsed"]').textContent = Math.floor(seconds / 60) + ':' + String(Math.floor(seconds % 60)).padStart(2, '0');
            find('[data-value="speed"]').textContent = formatBytes(state.bytes / seconds) + '/s';
            const skipped = Object.entries(state.skipped ?? {}).filter(([, count]) => count);
            const total = skipped.reduce((sum, [, count]) => sum + count, 0);
            for (const node of root.querySelectorAll('[data-skipped]')) node.hidden = !total;
            find('[data-value="skipped"]').textContent = total;
            find('.skipped').hidden = !total || busy || state.stage !== 'complete';
            find('.skipped dl').replaceChildren(...skipped.flatMap(([reason, count]) => {
                const term = document.createElement('dt'), value = document.createElement('dd');
                term.textContent = t(reason); value.textContent = count;
                return [term, value];
            }));
            const progress = find('progress');
            progress.hidden = !busy;
            if (state.total != null && !['verify', 'finishing'].includes(state.stage)) {
                progress.max = state.total || 1; progress.value = state.items;
            } else progress.removeAttribute('value');
        }
    }
    function errorStatus(error) {
        if (controller?.signal.aborted || error?.name === 'AbortError') return 'cancelled';
        if (error instanceof ExportError) return error.code;
        if (error?.name === 'QuotaExceededError') return 'quota';
        if (['NotAllowedError', 'SecurityError'].includes(error?.name)) return 'denied';
        return 'failed';
    }
    function controls() {
        button('start').disabled = busy || !find('input').checked || Boolean(url);
        button('cancel').hidden = !busy;
        button('close').disabled = busy || Boolean(url);
        button('cleanup').disabled = busy;
        button('persist').disabled = busy;
        find('input').disabled = busy;
    }
    find('input').onchange = controls;
    find('select').onchange = event => { lang = event.target.value; render(); };
    button('cancel').onclick = () => { controller?.abort(); button('cancel').disabled = true; };
    button('close').onclick = () => { if (!busy) { clearInterval(timer); host.remove(); } };
    button('download').onclick = () => {
        if (!url) return;
        const anchor = document.createElement('a');
        anchor.href = url; anchor.download = 'RisuNest-web-backup-' + new Date().toISOString().slice(0, 10) + '.bin';
        anchor.click(); status = 'downloadStarted'; render();
    };
    button('cleanup').onclick = async () => {
        if (busy || !window.confirm(t('cleanupConfirm'))) return;
        try {
            if (url) { URL.revokeObjectURL(url); url = null; }
            await clearTemporaryFiles();
            button('download').hidden = true; status = 'cleaned'; controls(); render();
        } catch (error) { status = errorStatus(error); render(); }
    };
    button('persist').onclick = async () => {
        status = 'permissionPending'; render();
        button('persist').disabled = true;
        try {
            status = await navigator.storage.persist() ? 'permissionGranted' : 'permissionDenied';
        } catch { status = 'permissionDenied'; }
        button('persist').disabled = false; render();
    };
    button('start').onclick = async () => {
        if (busy || !find('input').checked) return;
        busy = true; controller = new AbortController(); state = null; endTime = null; output = null;
        find('dl').hidden = true; find('.skipped').hidden = true; find('progress').hidden = true;
        button('cancel').disabled = false;
        find('.status').classList.remove('error');
        status = 'inventory'; controls(); render();
        // Invoke the picker before yielding the user's activation.
        let picker;
        try {
            checkPlatform();
            picker = temporary ? null : window.showSaveFilePicker({
                suggestedName: 'RisuNest-web-backup-' + new Date().toISOString().slice(0, 10) + '.bin',
                types: [{ description: 'RisuNest backup', accept: { 'application/octet-stream': ['.bin'] } }]
            });
            output = await destination(picker, controller.signal);
            check(controller.signal);
            timer = setInterval(render, 250);
            const source = await BrowserSource.open(controller.signal);
            await exportBackup({
                source, sink: output.sink, signal: controller.signal,
                progress(next) { state = next; status = next.stage; render(); }
            });
            if (output.temporary) {
                url = URL.createObjectURL(await output.file());
                button('download').hidden = false; status = 'prepared';
            } else status = 'complete';
        } catch (error) {
            status = errorStatus(error);
            if (status === 'quota' && navigator.storage?.persist) button('persist').hidden = false;
            find('.status').classList.toggle('error', status !== 'cancelled');
            await output?.sink.abort().catch(() => {});
            await output?.remove?.().catch(() => {});
        } finally {
            busy = false; endTime = performance.now();
            clearInterval(timer); controls(); render();
        }
    };
    render();
}
