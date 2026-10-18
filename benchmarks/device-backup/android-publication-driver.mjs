import assert from 'node:assert/strict';
import {execFile} from 'node:child_process';
import {promisify} from 'node:util';
import {mkdir, readFile, writeFile} from 'node:fs/promises';
import path from 'node:path';
import {androidTarget, runnerOptions} from '../support/androidTarget.mjs';
import {REALM_BLOCKED_URL_PATTERNS} from '../../scripts/realmBlocklist.mjs';

const options = runnerOptions(process.argv.slice(2));
const android = androidTarget(options);
const execute = promisify(execFile);
const output = path.resolve(options.output);
const mode = options.mode;
assert.ok(['terminal', 'copy', 'transport'].includes(mode), 'Select terminal, copy, or transport');
const packageName = 'io.github.rsyumi.risunest';
const title = 'RisuNest synthetic device backup smoke';
const port = Number(options.port ?? 19369);
const api = 'globalThis.__androidPublicationProbe';
const delay = (ms) => new Promise(resolve => setTimeout(resolve, ms));
const run = async (args) => (await execute(android.adb, android.args(args), {
  env: android.env, windowsHide: true, timeout: 15000, maxBuffer: 2 * 1024 * 1024,
})).stdout;
const report = {mode, serial: android.serial, avd: android.avd, startedAt: new Date().toISOString()};
const save = async () => { await mkdir(path.dirname(output), {recursive:true}); await writeFile(output, JSON.stringify(report, null, 2)); };
let client;

async function connect() {
  const pid = (await run(['shell','pidof',packageName])).trim();
  assert.match(pid, /^\d+$/);
  await run(['forward',`tcp:${port}`,`localabstract:webview_devtools_remote_${pid}`]);
  let target;
  for (let i = 0; i < 120; i++) {
    const pages = await fetch(`http://127.0.0.1:${port}/json/list`).then(r => r.json()).catch(() => []);
    target = pages.find(p => p.title === title && new URL(p.url).origin === 'http://tauri.localhost');
    if (target) break;
    await delay(250);
  }
  assert.ok(target, 'Separate synthetic harness page required');
  const url = new URL(target.webSocketDebuggerUrl);
  assert.ok(['localhost','127.0.0.1'].includes(url.hostname));
  assert.equal(Number(url.port), port);
  const socket = new WebSocket(url);
  await new Promise((resolve,reject) => { socket.onopen = resolve; socket.onerror = reject; });
  let sequence = 0, paused = false;
  const pending = new Map();
  socket.onmessage = ({data}) => {
    const message = JSON.parse(data);
    if (message.method === 'Debugger.paused') paused = true;
    const entry = pending.get(message.id);
    if (!entry) return;
    clearTimeout(entry.timer); pending.delete(message.id);
    if (message.error) entry.reject(new Error('CDP command failed'));
    else entry.resolve(message.result);
  };
  const call = (method,params={}) => new Promise((resolve,reject) => {
    const id=++sequence;
    const timer=setTimeout(() => {pending.delete(id);reject(new Error('CDP timeout'));},30000);
    pending.set(id,{resolve,reject,timer});socket.send(JSON.stringify({id,method,params}));
  });
  const evaluate = async (expression) => {
    const result=await call('Runtime.evaluate',{expression,awaitPromise:true,returnByValue:true});
    assert.ok(!result.exceptionDetails, 'Synthetic page evaluation failed');
    return result.result.value;
  };
  await call('Network.enable');
  await call('Network.setBlockedURLs',{urls:[...REALM_BLOCKED_URL_PATTERNS,'*update.rsyumi.workers.dev/translator/prompt-presets.json*']});
  assert.equal(await evaluate("window.__TAURI_INTERNALS__.invoke('plugin:app|identifier')"),packageName);
  assert.equal(await evaluate('document.title'),title);
  return {pid,evaluate,call,paused:()=>paused,close:()=>socket.close()};
}

async function tapPicker() {
  await run(['shell','uiautomator','dump','/data/local/tmp/risunest-publication-ui.xml']);
  const xml=await run(['shell','cat','/data/local/tmp/risunest-publication-ui.xml']);
  for (const node of xml.matchAll(/<node\b[^>]*>/g)) {
    const text=/\btext="([^"]*)"/.exec(node[0])?.[1];
    if (!['Save','SAVE','저장','Allow','ALLOW','허용','Replace','REPLACE'].includes(text)) continue;
    const bounds=/bounds="\[(\d+),(\d+)\]\[(\d+),(\d+)\]"/.exec(node[0]);
    if (!bounds) continue;
    await run(['shell','input','tap',String(Math.floor((+bounds[1]+ +bounds[3])/2)),String(Math.floor((+bounds[2]+ +bounds[4])/2))]);
    return text;
  }
  return false;
}

try {
  android.assertAvd(await run(['emu','avd','name']));
  const health=JSON.parse(await readFile(options.health,'utf8'));
  assert.equal(health.Serial,android.serial);
  assert.ok(health.Passed && health.Failures===0 && health.Seconds>=120 && health.Samples>=8 && health.HashMatch && health.Push && health.Pull && health.ConsolePing, 'Sustained emulator health required');
  client=await connect();
  report.pid=client.pid;
  report.before=await client.evaluate(`${api}.state()`);
  if (mode==='transport') {
    report.cases=[];
    for (const valueBytes of [1024,16384,32768]) {
      await client.evaluate(`void ${api}.transport(1024,512,${valueBytes}); true`);
      const deadline=Date.now()+120000;
      let state;
      do { await delay(250); state=await client.evaluate(`${api}.transportState()`); }
      while (state.phase==='running' && Date.now()<deadline);
      report.cases.push(state); await save();
      if (state.phase==='running') {
        report.nativeSlot=await client.evaluate(`${api}.nativeSlotAvailable()`); await save();
        throw new Error('Actual Android transport remained pending; inspect native slot evidence');
      }
      assert.equal(state.phase,'completed');
      assert.ok(state.firstCompleted && state.secondCompleted && state.uploadBytes===512*1024);
    }
  } else {
    assert.equal((await client.evaluate(`${api}.receipt()`)).intent,null,'No earlier publication may be pending');
    await client.call('Debugger.enable');
    await client.evaluate(`void ${api}.start(${Number(options.mib??32)},${JSON.stringify(mode)},true); true`);
    const deadline=Date.now()+180000;
    let lastPicker=0, pickerSaved=false;
    while (!client.paused() && Date.now()<deadline) {
      await delay(25);
      if (!pickerSaved && Date.now()-lastPicker>2000) {
        lastPicker=Date.now();const tapped=await tapPicker();pickerSaved=["Save","SAVE","저장","Replace","REPLACE"].includes(tapped);
      }
    }
    assert.ok(client.paused(),'Requested real SAF boundary was not observed');
    const record=JSON.parse(await run(['shell','run-as',packageName,'cat','files/native-file-jobs/android-saf-destination.json']));
    report.killRecord=record;
    await run(['shell','am','force-stop',packageName]);
    report.killedAt=Date.now();client.close();await save();
    assert.equal(record.phase,mode==='terminal'?'succeeded':'copying','Native state must match the requested kill boundary');
    await run(['shell','am','start','-n',`${packageName}/.MainActivity`]);
    await delay(15000);client=await connect();
    const recoveryDeadline=Date.now()+60000;
    let state;
    do {state=await client.evaluate(`${api}.state()`);if(state.dialogs.length)break;await delay(100);}while(Date.now()<recoveryDeadline);
    report.recovery=state;
    assert.equal(state.dialogs.length,1,'One product recovery dialog expected');
    assert.ok(!state.dialogs.some(d=>d.type==='error'),'Generic partial-file alert must remain silent');
    assert.ok(!state.dialogs.some(d=>d.message.includes('android-publication-retry-required')));
    assert.equal(state.dialogs[0].type,mode==='terminal'?'normal':'select');
    await client.evaluate(`${api}.closeDialog('1')`);
    const settledDeadline=Date.now()+15000;
    let receipt;
    do {await delay(100);receipt=await client.evaluate(`${api}.receipt()`);}while(receipt.intent && Date.now()<settledDeadline);
    report.after=await client.evaluate(`${api}.state()`);report.receipt=receipt;
    assert.equal(receipt.intent,null);assert.equal(receipt.status,null);
  }
  report.passed=true;
} catch(error) {report.error=error.message;process.exitCode=1;}
finally {await save();client?.close();await run(['forward','--remove',`tcp:${port}`]).catch(()=>{});}
