// Execute the shipped browser script to verify live, stale and offline rendering.
const fs = require('node:fs');
const vm = require('node:vm');
const assert = require('node:assert/strict');
const nodes = Object.fromEntries(['bpm','source','connection','mode','sleep','age'].map(id=>[id,{textContent:''}]));
const classes = new Set(); const timers = new Map(); let serial=0; let clock=1000; let watchdog;
const frames = new Map(); const listeners = {}; let path=[]; let drawn=[];
const canvas = {clientWidth:600,clientHeight:200,width:0,height:0,getContext:()=>({
  setTransform:()=>{},clearRect:()=>{},beginPath:()=>path=[],
  moveTo:(x,y)=>path.push([x,y]),lineTo:(x,y)=>path.push([x,y]),
  stroke:()=>{drawn=path;assert.equal(drawn.length,600,'trace memory stays bounded');}
})};
nodes.trace = canvas;
const motion = {matches:false,addEventListener:(event,fn)=>listeners.motion=fn};
let payload={available:true,heart_rate:75,mode:'hr_only',source:'HypeRate',demo:false,asleep:true,estimated:true,age_ms:0};
let fail=false; let calls=0; let concurrent=0;
const context={
  URLSearchParams,AbortController,location:{search:'?transparent=1&compact=1'},performance:{now:()=>clock},
  Float32Array,window:{devicePixelRatio:1,matchMedia:()=>motion,addEventListener:(event,fn)=>listeners[event]=fn},
  document:{hidden:false,addEventListener:(event,fn)=>listeners[event]=fn,getElementById:id=>nodes[id],body:{classList:{toggle:(key,on)=>on?classes.add(key):classes.delete(key),add:key=>classes.add(key),remove:key=>classes.delete(key)}}},
  requestAnimationFrame:fn=>{const id=++serial;frames.set(id,fn);assert.equal(frames.size,1);return id;},cancelAnimationFrame:id=>frames.delete(id),
  setTimeout:(fn,delay)=>{const id=++serial;timers.set(id,{fn,delay});return id;},clearTimeout:id=>timers.delete(id),setInterval:fn=>watchdog=fn,
  fetch:async(url,options)=>{assert.equal(url,'/api/v1/overlay/status');assert.equal(options.cache,'no-store');assert.equal(++concurrent,1);calls++;await Promise.resolve();concurrent--;if(fail)throw Error('offline');return {ok:true,json:async()=>payload};}
};
vm.runInNewContext(fs.readFileSync(__dirname+'/../crates/api/assets/heart-rate.js','utf8'),context);
const settle=()=>new Promise(resolve=>setImmediate(resolve));
async function nextPoll(){const entry=[...timers].find(([,timer])=>timer.delay===500);assert(entry,'poll should be scheduled');timers.delete(entry[0]);entry[1].fn();await settle();}
function advanceTrace(ms){for(let elapsed=0;elapsed<ms;elapsed+=10){clock+=10;const entry=[...frames][0];if(entry){frames.delete(entry[0]);entry[1](clock);}}}
function peaks(){return drawn.filter(([,y],i)=>y<70 && (i===0 || drawn[i-1][1]>=70)).length;}
function flat(){assert(drawn.every(([,y])=>Math.abs(y-112)<1e-6),'stopped trace should be a flat baseline');assert.equal(frames.size,0);}
(async()=>{
  await settle();assert.equal(nodes.bpm.textContent,'75');assert.equal(nodes.sleep.textContent,'擬似睡眠');assert(classes.has('transparent'));assert(classes.has('compact'));assert(classes.has('live'));
  advanceTrace(6000);assert(peaks()>=7 && peaks()<=8,'75 BPM should produce 7–8 pulses in six seconds');
  watchdog();assert.equal(nodes.bpm.textContent,'—');assert(!classes.has('live'));flat();
  fail=true;await nextPoll();assert.equal(nodes.bpm.textContent,'—');assert.equal(nodes.connection.textContent,'NemuriBeatに接続できません');
  fail=false;payload={...payload,heart_rate:82,mode:'hr_rr',estimated:false};await nextPoll();assert.equal(nodes.bpm.textContent,'82');assert.equal(nodes.sleep.textContent,'睡眠状態');
  payload={...payload,heart_rate:120};await nextPoll();advanceTrace(6000);assert.equal(peaks(),12,'120 BPM should produce 12 pulses in six seconds');
  context.document.hidden=true;listeners.visibilitychange();flat();
  await nextPoll();assert.equal(frames.size,0,'hidden browser must not animate');
  context.document.hidden=false;listeners.visibilitychange();await nextPoll();assert.equal(frames.size,1);
  motion.matches=true;listeners.motion();flat();await nextPoll();assert.equal(frames.size,0,'reduced motion must not animate');
  motion.matches=false;listeners.motion();assert.equal(frames.size,1);
  payload={...payload,available:false};await nextPoll();assert.equal(nodes.bpm.textContent,'—');assert.equal(nodes.sleep.textContent,'判定不能');
  flat();assert.equal(calls,8);console.log('PASS: browser live pulse, BPM-paced trace, bounded history, hidden/reduced-motion pause, stale/offline clearing, reconnect, sequential polling');
})().catch(error=>{console.error(error);process.exitCode=1;});
