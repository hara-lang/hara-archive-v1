import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readFile } from "node:fs/promises";
import test from "node:test";
import { BrowserPromiseProvider, decodeHta, encodeHta, HtaContext, HtaDeque, HtaHandle, HtaKeyword, HtaPriorityMap, HtaQueue, HtaSortedMap, HtaTagged, HtaSymbol, loadHtaExtension, parseHtaManifest } from "./packages/hta/index.js";

const tensorDescriptor='{:namespace "math.tensor" :version "1" :provider :wasm :module "tensor.wasm" :abi :hta.v1 :exports {"open" {:args [] :returns :value :async true}} :handles {"tensor" {:tag math}} :capabilities []}';
const hostDescriptor='{:namespace "host.demo" :version "1" :provider :wasm :module "demo.wasm" :abi :hta.v1 :exports {"open" {:args [] :returns :value}} :host-calls {"store" ["get"]} :capabilities []}';
const adapterDescriptor='{:namespace "math.async" :version "1" :provider :wasm :module "adapter.wasm" :abi :hta.v1 :exports {"sum" {:args [:i64 :i64] :returns :i64 :async true}} :assets ["adapter.wasm" "modules/math.wasm"] :capabilities []}';
const adapterFixtureDigest="6742ab577c2f6852103effd650d97d88c7427fd0e7520466126f892fb4fb0dab";
const libraryFixtureDigest="cf96c3351ea2afd66dd2cee4480ea44fd2e76f8009ca1df96edb9dc149749edc";

test("HTA0 browser codec matches the Java/Rust golden vector",()=>{assert.deepEqual([...encodeHta(["x",42,true])],[72,84,65,48,9,0,0,0,3,4,0,0,0,1,120,3,0,0,0,0,0,0,0,42,2]);assert.deepEqual(decodeHta(encodeHta(["x",42,true])),["x",42,true]);});
test("HTA0 preserves arbitrary-size integers as BigInt",()=>{const value=123456789012345678901234567890n;assert.equal(decodeHta(encodeHta(value)),value);assert.equal(decodeHta(encodeHta(-value)),-value);});
test("HTA0 rejects excessive nesting and impossible lengths",()=>{let value=null;for(let i=0;i<257;i++)value=[value];assert.throws(()=>encodeHta(value),/value-too-deep/);const deep=[72,84,65,48];for(let i=0;i<257;i++)deep.push(9,0,0,0,1);deep.push(0);assert.throws(()=>decodeHta(Uint8Array.from(deep)),/value-too-deep/);assert.throws(()=>decodeHta(Uint8Array.from([72,84,65,48,9,255,255,255,255])),/impossible sequence length/);});
test("HTA0 floats preserve IEEE-754 values",()=>{for(const value of [0.28,-0,Infinity,-Infinity,NaN]){const decoded=decodeHta(encodeHta(value));if(Number.isNaN(value))assert.ok(Number.isNaN(decoded));else assert.ok(Object.is(decoded,value));}});
test("opaque handles round trip canonically",()=>{const value=new HtaHandle("runtime","cursor",42n);const decoded=decodeHta(encodeHta(value));assert.equal(decoded.owner,"runtime");assert.equal(decoded.type,"cursor");assert.equal(decoded.id,42n);assert.equal(decoded.toString(),"#ht[:handle 42]");});
test("canonical maps ignore insertion order",()=>{const a=new Map([[new HtaKeyword("b"),2],[new HtaKeyword("a"),1]]),b=new Map([[new HtaKeyword("a"),1],[new HtaKeyword("b"),2]]);assert.deepEqual(encodeHta(a),encodeHta(b));});
test("HTA v3 preserves immutable Hara collection and tagged identities",()=>{const values=[new HtaQueue([1,2]),new HtaDeque([1,2]),new HtaSortedMap([["a",1],["b",2]]),new HtaPriorityMap([["b",1],["a",2]]),new HtaTagged(new HtaSymbol("demo/tag"),42)];for(const value of values){const decoded=decodeHta(encodeHta(value));assert.equal(decoded.constructor,value.constructor);assert.deepEqual(decoded,value);}});
test("context applies registered public handle tags",async()=>{const worker=new FakeWorker();const context=new HtaContext({worker,moduleUrl:"runtime.wasm",handleTags:{tensor:"math"}});worker.emit({type:"ready"});const result=context.call("open",[]);await Promise.resolve();const call=worker.sent.find(message=>message.type==="call");worker.emit({type:"result",id:call.id,ok:true,frame:encodeHta(new HtaHandle("math.tensor","tensor",42n))});assert.equal(String(await result),"#math[:tensor 42]");context.close();});
test("context forwards provider instrumentation without changing transport",async()=>{const worker=new FakeWorker(),events=[];const context=new HtaContext({worker,moduleUrl:"runtime.wasm",instrumentation:true,onProviderEvent:event=>events.push(event)});assert.equal(worker.sent[0].instrumentation,true);worker.emit({type:"provider-event",event:{schema:"hara.hta.provider.event/0-alpha",sequence:1,event:"start"}});assert.deepEqual(events,[{schema:"hara.hta.provider.event/0-alpha",sequence:1,event:"start"}]);context.close();});
test("manifest parser validates compact public tags",()=>{const manifest=parseHtaManifest(tensorDescriptor);assert.equal(manifest.namespace,"math.tensor");assert.equal(manifest.module,"tensor.wasm");assert.deepEqual(manifest.handleTags,{tensor:"math"});assert.throws(()=>parseHtaManifest(tensorDescriptor.replace(":tag math",":tag Math")),/invalid handle tag/);});
test("manifest parser preserves export and host-call policy",()=>{const manifest=parseHtaManifest(hostDescriptor);assert.deepEqual(manifest.exports,["open"]);assert.deepEqual(manifest.hostCalls,{store:["get"]});assert.throws(()=>parseHtaManifest(hostDescriptor.replace("[\"get\"]","[\"get/x\"]")),/invalid host-call/);});
test("manifest parser enforces declared HTA targets and typed exports",()=>{
const descriptor='{:namespace "demo.hta" :version "1" :provider :hta :abi :hta.v1 :targets {:node {:provider "node/provider.mjs" :runtime :process} :browser {:provider "browser/provider.mjs" :runtime :web-worker}} :exports {"open" {:args [:value] :returns :value :async true}} :capabilities []}';
  const manifest=parseHtaManifest(descriptor);
  assert.equal(manifest.version,"1");
  assert.deepEqual(manifest.targets,{node:{provider:"node/provider.mjs",runtime:"process"},browser:{provider:"browser/provider.mjs",runtime:"web-worker"}});
  assert.deepEqual(manifest.exportSpecs.open.args,[new HtaKeyword("value")]);
  assert.equal(manifest.exportSpecs.open.async,true);
  assert.throws(()=>parseHtaManifest(descriptor.replace(":returns :value",":returns nil")),/invalid export returns/);
  assert.throws(()=>parseHtaManifest(descriptor.replace(":browser {:provider \"browser/provider.mjs\" :runtime :web-worker}","")),/browser web-worker target/);
  assert.throws(()=>parseHtaManifest(descriptor.replace(":version \"1\"",":version nil")),/invalid version/);
  assert.throws(()=>parseHtaManifest(descriptor.replace(":provider \"browser/provider.mjs\"",":module \"browser/worker.mjs\"")),/invalid browser target/);
  assert.throws(()=>parseHtaManifest(descriptor.replace(":browser {:provider \"browser/provider.mjs\" :runtime :web-worker}",":browser {:provider \"browser/provider.mjs\" :runtime :web-worker :module \"browser/worker.mjs\"}")),/invalid browser target/);
  assert.throws(()=>parseHtaManifest(descriptor.replace(":provider :hta",":provider :hta :worker \"browser/worker.mjs\"")),/unknown manifest field/);
});
test("descriptor loader resolves wasm and applies handle tags",async()=>{const worker=new FakeWorker();const context=await loadHtaExtension({worker,descriptor:tensorDescriptor,packageUrl:"https://example.test/extensions/math/"});assert.equal(worker.sent[0].moduleUrl,"https://example.test/extensions/math/tensor.wasm");worker.emit({type:"ready"});const result=context.call("open",[]);await Promise.resolve();const call=worker.sent.find(message=>message.type==="call");worker.emit({type:"result",id:call.id,ok:true,frame:encodeHta(new HtaHandle("math.tensor","tensor",42n))});assert.equal(String(await result),"#math[:tensor 42]");context.close();});
test("descriptor loader resolves the wrapped library for generated adapters",async()=>{const worker=new FakeWorker();const context=await loadHtaExtension({worker,descriptor:adapterDescriptor,packageUrl:"https://example.test/extensions/math/"});assert.equal(worker.sent[0].moduleUrl,"https://example.test/extensions/math/adapter.wasm");assert.equal(worker.sent[0].libraryUrl,"https://example.test/extensions/math/modules/math.wasm");context.close();});
test("worker composes a generated HTA adapter with its wrapped library",async()=>{
  const adapterBytes=new Uint8Array(await readFile(new URL("./test-fixtures/hta-adapter/adapter.wasm",import.meta.url)));
  const libraryBytes=new Uint8Array(await readFile(new URL("./test-fixtures/hta-adapter/library.wasm",import.meta.url)));
  const messages=[],listeners=new Map(),previousSelf=globalThis.self;
  globalThis.self={addEventListener:(type,handler)=>listeners.set(type,handler),postMessage:message=>messages.push(message),close:()=>{}};
  try{
    await import(`./packages/hta/worker.mjs?hta-composition=${Date.now()}`);
    const message=listeners.get("message");
    await message({data:{type:"init",moduleBytes:adapterBytes,libraryBytes}});
    assert.deepEqual(messages,[{type:"ready"}]);
    await message({data:{type:"call",id:1,frame:encodeHta(["sum",[19,23]])}});
    const result=messages.find(item=>item.type==="result");
    assert.equal(result.id,1);
    assert.equal(result.ok,true);
    assert.equal(decodeHta(result.frame),42);
  }finally{
    if(previousSelf===undefined)delete globalThis.self;else globalThis.self=previousSelf;
  }
});
test("generic Wasm worker exposes the provider lifecycle vocabulary",async()=>{
  const adapterBytes=new Uint8Array(await readFile(new URL("./test-fixtures/hta-adapter/adapter.wasm",import.meta.url)));
  const libraryBytes=new Uint8Array(await readFile(new URL("./test-fixtures/hta-adapter/library.wasm",import.meta.url)));
  const messages=[],listeners=new Map(),previousSelf=globalThis.self;
  globalThis.self={addEventListener:(type,handler)=>listeners.set(type,handler),postMessage:message=>messages.push(message),close:()=>{}};
  try{
    await import(`./packages/hta/worker.mjs?hta-instrumentation=${Date.now()}`);
    const message=listeners.get("message");
    await message({data:{type:"init",moduleBytes:adapterBytes,libraryBytes,instrumentation:true}});
    await message({data:{type:"call",id:3,frame:encodeHta(["sum",[19,23]])}});
    await message({data:{type:"close"}});
    assert.deepEqual(messages.filter(item=>item.type==="provider-event").map(item=>item.event.event),["start","call-enter","call-return","terminal","shutdown"]);
  }finally{
    if(previousSelf===undefined)delete globalThis.self;else globalThis.self=previousSelf;
  }
});
test("JVM and browser composition fixtures retain their reviewed bytes",async()=>{
  const adapterBytes=await readFile(new URL("./test-fixtures/hta-adapter/adapter.wasm",import.meta.url));
  const libraryBytes=await readFile(new URL("./test-fixtures/hta-adapter/library.wasm",import.meta.url));
  assert.equal(createHash("sha256").update(adapterBytes).digest("hex"),adapterFixtureDigest);
  assert.equal(createHash("sha256").update(libraryBytes).digest("hex"),libraryFixtureDigest);
});
test("worker rejects a wrapped library with a missing imported export",async()=>{
  const adapterBytes=new Uint8Array(await readFile(new URL("./test-fixtures/hta-adapter/adapter.wasm",import.meta.url)));
  const libraryBytes=replaceUtf8(await readFile(new URL("./test-fixtures/hta-adapter/library.wasm",import.meta.url)),"add","sub");
  const messages=[],listeners=new Map(),previousSelf=globalThis.self;
  globalThis.self={addEventListener:(type,handler)=>listeners.set(type,handler),postMessage:message=>messages.push(message),close:()=>{}};
  try{
    await import(`./packages/hta/worker.mjs?hta-malformed=${Date.now()}`);
    await listeners.get("message")({data:{type:"init",moduleBytes:adapterBytes,libraryBytes}});
    assert.equal(messages[0].type,"fatal");
    assert.match(messages[0].error.message,/hta\/library-export-missing/);
  }finally{
    if(previousSelf===undefined)delete globalThis.self;else globalThis.self=previousSelf;
  }
});
test("generic worker loads a declared provider implementation",async()=>{
  const messages=[],listeners=new Map(),previousSelf=globalThis.self;
  globalThis.self={addEventListener:(type,handler)=>listeners.set(type,handler),postMessage:message=>messages.push(message),close:()=>{}};
  const providerUrl=`data:text/javascript,${encodeURIComponent("export default async (operation,args)=>operation === 'sum' ? args[0] + args[1] : null")}`;
  try{
    await import(`./packages/hta/worker.mjs?hta-provider=${Date.now()}`);
    const message=listeners.get("message");
    await message({data:{type:"init",backend:"provider",providerUrl}});
    assert.deepEqual(messages,[{type:"ready"}]);
    await message({data:{type:"call",id:7,frame:encodeHta(["sum",[19,23]])}});
    const result=messages.find(item=>item.type==="result");
    assert.equal(result.id,7);
    assert.equal(result.ok,true);
    assert.equal(decodeHta(result.frame),42);
  }finally{
    if(previousSelf===undefined)delete globalThis.self;else globalThis.self=previousSelf;
  }
});
test("generic worker closes a provider through its declared cleanup export",async()=>{
  const messages=[],listeners=new Map(),previousSelf=globalThis.self;
  const marker=`__htaProviderClosed${Date.now()}`;
  let closed=false;
  globalThis[marker]=false;
  globalThis.self={addEventListener:(type,handler)=>listeners.set(type,handler),postMessage:message=>messages.push(message),close:()=>{closed=true;}};
  const providerUrl=`data:text/javascript,${encodeURIComponent(`export default async () => null; export const close = () => { globalThis.${marker} = true; };`)}`;
  try{
    await import(`./packages/hta/worker.mjs?hta-provider-close=${Date.now()}`);
    const message=listeners.get("message");
    await message({data:{type:"init",backend:"provider",providerUrl}});
    await message({data:{type:"close"}});
    assert.equal(globalThis[marker],true);
    assert.equal(closed,true);
  }finally{
    delete globalThis[marker];
    if(previousSelf===undefined)delete globalThis.self;else globalThis.self=previousSelf;
  }
});
test("generic worker releases provider handles through the declared hook",async()=>{
  const messages=[],listeners=new Map(),previousSelf=globalThis.self;
  const marker=`__htaProviderReleased${Date.now()}`;
  globalThis[marker]=null;
  globalThis.self={addEventListener:(type,handler)=>listeners.set(type,handler),postMessage:message=>messages.push(message),close:()=>{}};
  const providerUrl=`data:text/javascript,${encodeURIComponent(`export default async () => ({owner: 'demo', type: 'cursor', id: 42n}); export const release = handle => { globalThis.${marker} = handle; };`)}`;
  try{
    await import(`./packages/hta/worker.mjs?hta-provider-release=${Date.now()}`);
    const message=listeners.get("message");
    await message({data:{type:"init",backend:"provider",providerUrl}});
    await message({data:{type:"release",frame:encodeHta(new HtaHandle("demo","cursor",42n))}});
    assert.equal(globalThis[marker].owner,"demo");
    assert.equal(globalThis[marker].type,"cursor");
    assert.equal(globalThis[marker].id,42n);
  }finally{
    delete globalThis[marker];
    if(previousSelf===undefined)delete globalThis.self;else globalThis.self=previousSelf;
  }
});
test("descriptor loader fetches EDN when given its URL",async()=>{const worker=new FakeWorker(),descriptorUrl=`data:text/plain,${encodeURIComponent(tensorDescriptor)}`;const context=await loadHtaExtension({worker,descriptorUrl,moduleBytes:new Uint8Array()});assert.deepEqual(context.manifest.handleTags,{tensor:"math"});assert.ok(worker.sent[0].moduleBytes instanceof Uint8Array);context.close();});
test("context releases bound handles once and rejects later use",async()=>{const worker=new FakeWorker();const context=new HtaContext({worker,moduleUrl:"runtime.wasm"});worker.emit({type:"ready"});const result=context.call("open",[]);await Promise.resolve();const call=worker.sent.find(message=>message.type==="call");worker.emit({type:"result",id:call.id,ok:true,frame:encodeHta(new HtaHandle("runtime","cursor",42n))});const handle=await result;handle.release();handle.release();const releases=worker.sent.filter(message=>message.type==="release");assert.equal(releases.length,1);const released=decodeHta(releases[0].frame);assert.equal(released.id,42n);await assert.rejects(context.call("use",[handle]),/hta\/handle-released/);context.close();});
test("context exposes worker results as promises",async()=>{const worker=new FakeWorker();const context=new HtaContext({worker,moduleUrl:"runtime.wasm"});worker.emit({type:"ready"});const result=context.call("eval",["(+ 1 2)"]);await Promise.resolve();const call=worker.sent.find(message=>message.type==="call");worker.emit({type:"result",id:call.id,ok:true,frame:encodeHta(3)});assert.equal(await result,3);context.close();});
test("context cancellation does not leak pre-dispatch requests",async()=>{const worker=new FakeWorker();const context=new HtaContext({worker,moduleUrl:"runtime.wasm"});worker.emit({type:"ready"});const result=context.call("eval",["slow"]);const rejection=assert.rejects(result,/cancelled/);result.cancel();await Promise.resolve();await Promise.resolve();assert.equal(worker.sent.some(message=>message.type==="call"),false);assert.equal(context.pending.size,0);await rejection;context.close();});
test("context cancellation is forwarded after dispatch",async()=>{const worker=new FakeWorker();const context=new HtaContext({worker,moduleUrl:"runtime.wasm"});worker.emit({type:"ready"});const result=context.call("eval",["slow"]);const rejection=assert.rejects(result,/cancelled/);await Promise.resolve();await Promise.resolve();result.cancel();assert.equal(worker.sent.at(-1).type,"cancel");assert.equal(context.pending.size,0);await rejection;context.close();});
test("context failure rejects pending calls and terminates the worker",async()=>{
const worker=new FakeWorker(),context=new HtaContext({worker,moduleUrl:"runtime.wasm"});
worker.emit({type:"ready"});
const pending=context.call("slow");
await Promise.resolve();await Promise.resolve();
worker.emit({type:"fatal",error:{message:"malformed HTA module"}});
await assert.rejects(pending,/malformed HTA module/);
assert.equal(context.pending.size,0);
assert.equal(worker.terminated,true);
context.close();});
test("context enforces manifest export and host-call policy",async()=>{
  const worker=new FakeWorker(),calls={"store/get":async()=>42,"store/put":async()=>false};
  const context=new HtaContext({worker,moduleUrl:"runtime.wasm",hostCalls:calls,manifest:parseHtaManifest(hostDescriptor)});
  worker.emit({type:"ready"});
  await assert.rejects(context.call("missing"),/hta\/export-denied/);
  worker.emit({type:"host-call",service:"store",method:"put",call:1,frame:encodeHta([])});
  const denied=decodeHta(worker.sent.at(-1).frame);
  assert.equal([...denied].find(([key])=>key instanceof HtaKeyword && key.name==="message")[1],"hta/host-call-denied: store/put");
  worker.emit({type:"host-call",service:"store",method:"get",call:2,frame:encodeHta([])});
  await new Promise(resolve=>setTimeout(resolve,0));
  assert.equal(worker.sent.at(-1).ok,true);
  await context.close();
});
test("context cancellation aborts in-flight Wasm host calls",async()=>{
  const worker=new FakeWorker();let aborted=false;
  const context=new HtaContext({worker,moduleUrl:"runtime.wasm",hostCalls:{"store/get":async function(){return new Promise((resolve,reject)=>{this.signal.addEventListener("abort",()=>{aborted=true;reject(new Error("cancelled"));},{once:true});});}}});
  worker.emit({type:"ready"});
  worker.emit({type:"host-call",call:11,task:21,service:"store",method:"get",frame:encodeHta([])});
  await Promise.resolve();
  worker.emit({type:"host-cancel",calls:[{call:11,task:21}]});
  await new Promise(resolve=>setTimeout(resolve,0));
  assert.equal(aborted,true);
  await context.close();
});
test("context close aborts in-flight Wasm host calls",async()=>{
  const worker=new FakeWorker();let aborted=false;
  const context=new HtaContext({worker,moduleUrl:"runtime.wasm",hostCalls:{"store/get":async function(){return new Promise((resolve,reject)=>{this.signal.addEventListener("abort",()=>{aborted=true;reject(new Error("closed"));},{once:true});});}}});
  worker.emit({type:"ready"});
  worker.emit({type:"host-call",call:12,task:22,service:"store",method:"get",frame:encodeHta([])});
  await Promise.resolve();
  await context.close();
  assert.equal(aborted,true);
  assert.equal(context.hostCallsInFlight.size,0);
});
test("context close rejects pending calls and is idempotent",async()=>{
  const worker=new FakeWorker(),context=new HtaContext({worker,moduleUrl:"runtime.wasm"});
  worker.emit({type:"ready"});
  const pending=context.call("slow");
  await Promise.resolve();await Promise.resolve();
  const first=context.close(),second=context.close();
  await assert.rejects(pending,/hta\/context-closed/);
  await Promise.all([first,second]);
  assert.equal(worker.terminated,true);
  assert.equal(context.pending.size,0);
});
test("context registers kernel-issued mounts and sessions attach numeric ids",async()=>{
  const worker=new FakeWorker(),events=[];
  const filesystemHost={register:async(_context,id,descriptor)=>events.push(["register",id,descriptor]),close:async(_context,id)=>events.push(["close",id])};
  const context=new HtaContext({worker,moduleUrl:"runtime.wasm",filesystemHost});
  worker.emit({type:"ready"});
  const creating=context.createFilesystem({provider:"memory"});
  await Promise.resolve();await Promise.resolve();
  let call=worker.sent.find(message=>message.type==="call");
  assert.equal(decodeHta(call.frame)[0],"filesystem/create");
  worker.emit({type:"result",id:call.id,ok:true,frame:encodeHta(7)});
  assert.equal(await creating,7);
  assert.deepEqual(events,[["register",7,{provider:"memory"}]]);
  const attaching=context.session("alpha").attachFilesystem(7);
  await Promise.resolve();await Promise.resolve();
  call=worker.sent.filter(message=>message.type==="call").at(-1);
  assert.deepEqual(decodeHta(call.frame),["session/attach-filesystem",["alpha",7]]);
  worker.emit({type:"result",id:call.id,ok:true,frame:encodeHta(true)});
  assert.equal(await attaching,true);
  const closing=context.closeFilesystem(7);
  await Promise.resolve();await Promise.resolve();
  call=worker.sent.filter(message=>message.type==="call").at(-1);
  worker.emit({type:"result",id:call.id,ok:true,frame:encodeHta(true)});
  assert.equal(await closing,true);
  assert.deepEqual(events.at(-1),["close",7]);
  context.close();
});

class FakeWorker{constructor(){this.listeners={};this.sent=[];}addEventListener(type,handler){this.listeners[type]=handler;}postMessage(message){this.sent.push(message);if(message.type==="close")queueMicrotask(()=>this.emit({type:"closed"}));}emit(data){this.listeners.message({data});}terminate(){this.terminated=true;}}

function replaceUtf8(bytes,from,to){const source=new TextEncoder().encode(from),replacement=new TextEncoder().encode(to);if(source.length!==replacement.length)throw new Error("fixture lengths differ");const copy=bytes.slice();for(let index=0;index<=copy.length-source.length;index++){let match=true;for(let offset=0;offset<source.length;offset++)if(copy[index+offset]!==source[offset]){match=false;break;}if(match){copy.set(replacement,index);return copy;}}throw new Error(`fixture marker missing: ${from}`);}

test("browser promise provider uses native microtasks and ordered chaining",async()=>{
  const provider=new BrowserPromiseProvider(),events=[];
  const source=provider.run(()=>{events.push("run");return 20;});
  provider.then(source,value=>{events.push("first");return value+1;});
  const result=provider.then(source,value=>{events.push("second");return provider.run(()=>value*2);});
  events.push("sync");
  assert.equal(await result,40);
  assert.deepEqual(events,["sync","run","first","second"]);
});

test("browser promise provider adopts, recovers, finalizes, orders all, and settles once",async()=>{
  const provider=new BrowserPromiseProvider(),events=[];
  const adopted=provider.run(()=>provider.run(()=>7));
  const recovered=provider.catch(provider.run(()=>{throw new Error("broken");}),error=>error.message);
  const finalized=provider.finally(adopted,()=>{events.push("finally");});
  assert.deepEqual(await provider.all([recovered,finalized,3]),["broken",7,3]);
  assert.deepEqual(events,["finally"]);
  let resolveSource,rejectSource;
  const once=provider.create((resolve,reject)=>{resolveSource=resolve;rejectSource=reject;});
  assert.equal(resolveSource(1),true);assert.equal(rejectSource(new Error("late")),false);assert.equal(await once,1);
});

test("browser promise provider cancellation prevents deferred work",async()=>{
  const scheduled=[];
  const provider=new BrowserPromiseProvider({schedule:(task)=>{scheduled.push(task);return 0;},cancelSchedule:()=>scheduled.splice(0),enqueue:queueMicrotask});
  let ran=false;const delayed=provider.delay(10,()=>{ran=true;return 1;});
  assert.equal(delayed.cancel(),true);assert.equal(delayed.cancel(),false);
  await assert.rejects(delayed,/cancelled/);assert.equal(ran,false);assert.equal(scheduled.length,0);
});
