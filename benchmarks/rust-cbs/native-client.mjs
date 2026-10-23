import { spawn } from 'node:child_process';
import { createInterface } from 'node:readline';
export function nativeClient(binary) {
  const child=spawn(binary,[],{stdio:['pipe','pipe','ignore'],windowsHide:true});
  const lines=createInterface({input:child.stdout}); let pending;
  lines.on('line',line=>{if(pending){const p=pending;pending=undefined;clearTimeout(p.timer);p.resolve(JSON.parse(line));}});
  child.on('exit',()=>{if(pending){clearTimeout(pending.timer);pending.resolve({error:'process_exit'});pending=undefined;}});
  return { async send(request,timeout=3000) {
    if(pending)throw Error('Concurrent native request');
    return new Promise(resolve=>{pending={resolve,timer:setTimeout(()=>{pending=undefined;child.kill();resolve({error:'timeout'});},timeout)};child.stdin.write(JSON.stringify(request)+'\n');});
  },close(){lines.close();child.stdin.end();child.kill();} };
}
