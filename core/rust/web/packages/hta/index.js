const MAGIC = new Uint8Array([0x48, 0x54, 0x41, 0x30]);
const TAG = { nil: 0, false: 1, true: 2, i64: 3, string: 4, bytes: 5, keyword: 6, symbol: 7, list: 8, vector: 9, set: 10, map: 11, handle: 12, namespace: 13, var: 14, f64: 15, atom: 16, array: 17, object: 18, bigInteger: 20, tuple:23, cons:24, queue:25, orderedMap:26, sortedMap:27, trie:28, orderedSet:29, sortedSet:30, tagged:31, exceptionInfo:32, struct:33, deque:36, priorityMap:37 };
const encoder = new TextEncoder();
const decoder = new TextDecoder("utf-8", { fatal: true });
export const HTA_MAX_FRAME_BYTES = 64 * 1024 * 1024;
export const HTA_MAX_NESTING_DEPTH = 256;

export class HtaKeyword { constructor(name) { this.name = name; } }
export class HtaSymbol { constructor(name) { this.name = name; } }
export class HtaVar { constructor(symbol,value=null){this.symbol=symbol;this.value=value;} toString(){return `#'${this.symbol.name}`;} }
export class HtaAtom { constructor(value){this.value=value;} toString(){return `#atom <${displayHta(this.value)}>`;} }
export class HtaArray { constructor(values){this.values=values;} toString(){return `(array${this.values.length?` ${this.values.map(displayHta).join(" ")}`:""})`;} }
export class HtaObject { constructor(entries){this.entries=entries;} toString(){return `(object${this.entries.length?` ${this.entries.map(([key,value])=>`${JSON.stringify(key)} ${displayHta(value)}`).join(" ")}`:""})`;} }
export class HtaTuple { constructor(values){this.values=values;} }
export class HtaCons { constructor(values){this.values=values;} }
export class HtaQueue { constructor(values){this.values=values;} }
export class HtaDeque { constructor(values){this.values=values;} }
export class HtaOrderedMap { constructor(entries){this.entries=entries;} }
export class HtaSortedMap { constructor(entries){this.entries=entries;} }
export class HtaTrie { constructor(entries){this.entries=entries;} }
export class HtaPriorityMap { constructor(entries){this.entries=entries;} }
export class HtaOrderedSet { constructor(values){this.values=values;} }
export class HtaSortedSet { constructor(values){this.values=values;} }
export class HtaTagged { constructor(tag,value){this.tag=tag;this.value=value;} }
export class HtaExceptionInfo { constructor(message,data,cause=null){this.message=message;this.data=data;this.cause=cause;} }
export class HtaStruct { constructor(name,fields,values){this.name=name;this.fields=fields;this.values=values;} }
function displayHta(value){if(value===null)return"nil";if(typeof value==="string")return JSON.stringify(value);if(value instanceof HtaKeyword)return`:${value.name}`;if(value instanceof Map)return`{${[...value].map(([key,item])=>`${displayHta(key)} ${displayHta(item)}`).join(" ")}}`;if(Array.isArray(value))return`[${value.map(displayHta).join(" ")}]`;return String(value);}
export class HtaHandle { constructor(owner,type,id,context=null,displayTag="ht",displayKind="handle"){this.owner=owner;this.type=type;this.id=BigInt(id);this.context=context;this.displayTag=displayTag;this.displayKind=displayKind;this.released=false;} release(){if(this.released)return;this.released=true;if(this.context)this.context.releaseHandle(this);} toString(){return `#${this.displayTag}[:${this.displayKind} ${this.id}]`;} }

/** Browser host adapter for the portable Hara promise-provider contract. */
export class BrowserPromiseProvider {
  constructor(options={}) {
    this.enqueue=options.enqueue ?? (task=>queueMicrotask(task));
    this.schedule=options.schedule ?? ((task,milliseconds)=>setTimeout(task,milliseconds));
    this.cancelSchedule=options.cancelSchedule ?? (timer=>clearTimeout(timer));
  }
  create(executor) {
    let settled=false,rejectPromise=()=>{},cancelAction=()=>{};
    const promise=new Promise((resolve,reject)=>{
      rejectPromise=reject;
      const settle=(callback)=>(value)=>{if(settled)return false;settled=true;callback(value);return true;};
      const onCancel=(action)=>{cancelAction=typeof action==="function"?action:()=>{};};
      try{executor(settle(resolve),settle(reject),onCancel);}catch(error){settle(reject)(error);}
    });
    promise.cancel=()=>{if(settled)return false;cancelAction();settled=true;rejectPromise(new Error("cancelled"));return true;};
    return promise;
  }
  run(task) {
    return this.create((resolve,reject,onCancel)=>{
      let cancelled=false;onCancel(()=>{cancelled=true;});
      this.enqueue(()=>{if(cancelled)return;try{resolve(task());}catch(error){reject(error);}});
    });
  }
  delay(milliseconds,task) {
    return this.create((resolve,reject,onCancel)=>{
      const timer=this.schedule(()=>{try{resolve(task());}catch(error){reject(error);}},milliseconds);
      onCancel(()=>this.cancelSchedule(timer));
    });
  }
  all(values) { return this.create((resolve,reject)=>Promise.all(values).then(resolve,reject)); }
  then(source,callback) { return this.create((resolve,reject)=>Promise.resolve(source).then(callback).then(resolve,reject)); }
  catch(source,callback) { return this.create((resolve,reject)=>Promise.resolve(source).catch(callback).then(resolve,reject)); }
  finally(source,callback) { return this.create((resolve,reject)=>Promise.resolve(source).finally(callback).then(resolve,reject)); }
}

export function encodeHta(value) {
  const output = [...MAGIC];
  writeValue(output, value, 0);
  if(output.length>HTA_MAX_FRAME_BYTES)throw new Error("hta/value-too-large: frame exceeds 64 MiB");
  return Uint8Array.from(output);
}

export function decodeHta(input) {
  const bytes = input instanceof Uint8Array ? input : new Uint8Array(input);
  if(bytes.length>HTA_MAX_FRAME_BYTES)throw new Error("hta/value-too-large: frame exceeds 64 MiB");
  if (bytes.length < 4 || !MAGIC.every((byte, index) => bytes[index] === byte)) {
    throw new Error("hta/value-malformed: invalid HTA0 header");
  }
  const reader = new Reader(bytes, 4);
  const value = reader.value(0);
  if (reader.cursor !== bytes.length) throw new Error("hta/value-malformed: trailing bytes");
  return value;
}

export function parseHtaManifest(source) {
  const value = parseEdnData(source, "hta/manifest-malformed");
  if (!(value instanceof Map)) throw new Error("hta/manifest-malformed: expected one EDN map");
  const namespace = manifestField(value,"namespace"), identity = manifestField(value,"identity"), providerValue = manifestField(value,"provider"), module = manifestField(value,"module"), abiValue = manifestField(value,"abi");
  if (typeof namespace !== "string" || !/^[a-z][a-z0-9-]*(\.[a-z0-9][a-z0-9-]*)+$/.test(namespace)) throw new Error("hta/manifest-malformed: invalid namespace");
  if (identity !== undefined && (typeof identity !== "string" || !/^[a-z][a-z0-9-]*\/[a-z][a-z0-9-]*(\.[a-z0-9][a-z0-9-]*)*$/.test(identity))) throw new Error("hta/manifest-malformed: invalid identity");
  if (!(providerValue instanceof HtaKeyword) || !["wasm","hta"].includes(providerValue.name)) throw new Error("hta/manifest-malformed: provider must be :wasm or :hta");
  if (!(abiValue instanceof HtaKeyword)) throw new Error("hta/manifest-malformed: abi must be a keyword");
  const provider=providerValue.name,abi=abiValue.name;
  let browserTarget;
  if (provider === "wasm") {
    if (!validPackagePath(module,".wasm")) throw new Error("hta/manifest-malformed: invalid module");
  } else {
    if (module !== undefined || abi !== "hta.v1") throw new Error("hta/manifest-malformed: HTA targets require :abi :hta.v1 without :module");
    const targets=manifestField(value,"targets"),browser=targets instanceof Map?manifestField(targets,"browser"):undefined;
    const browserModule=browser instanceof Map?manifestField(browser,"module"):undefined;
    const runtime=browser instanceof Map?manifestField(browser,"runtime"):undefined;
    if (!validPackagePath(browserModule,".mjs") || !(runtime instanceof HtaKeyword) || runtime.name !== "web-worker") throw new Error("hta/manifest-malformed: missing :browser web-worker target");
    browserTarget=Object.freeze({module:browserModule,runtime:runtime.name});
  }
  const assetsValue=manifestField(value,"assets"),assets=[];
  if (assetsValue !== undefined) {
    if (!Array.isArray(assetsValue) || assetsValue.some(asset=>!validPackagePath(asset))) throw new Error("hta/manifest-malformed: invalid assets");
    assets.push(...assetsValue);
  }
  const handleTags = {}, handles = manifestField(value,"handles");
  if (handles !== undefined) {
    if (!(handles instanceof Map)) throw new Error("hta/manifest-malformed: handles must be a map");
    for (const [type,spec] of handles) {
      const tag = spec instanceof Map ? manifestField(spec,"tag") : undefined;
      if (typeof type !== "string" || !/^[a-z][a-z0-9-]*$/.test(type) || !(tag instanceof HtaSymbol) || !/^[a-z][a-z0-9-]*(\.[a-z][a-z0-9-]*)*$/.test(tag.name)) throw new Error("hta/manifest-malformed: invalid handle tag");
      handleTags[type] = tag.name;
    }
  }
  const exportsValue = manifestField(value,"exports"), exports = [];
  if (exportsValue !== undefined) {
    if (!(exportsValue instanceof Map)) throw new Error("hta/manifest-malformed: exports must be a map");
    for (const [name,spec] of exportsValue) {
      if (typeof name !== "string" || !name.length || !(spec instanceof Map)) throw new Error("hta/manifest-malformed: invalid export");
      exports.push(name);
    }
  }
  const hostCallsValue = manifestField(value,"host-calls"), hostCalls = {};
  if (hostCallsValue !== undefined) {
    if (!(hostCallsValue instanceof Map)) throw new Error("hta/manifest-malformed: host-calls must be a map");
    for (const [service,methods] of hostCallsValue) {
      if (typeof service !== "string" || !/^[a-z][a-z0-9.-]*$/.test(service) || !Array.isArray(methods) ||
          methods.some(method => typeof method !== "string" || !/^[a-z][a-z0-9-]*$/.test(method))) {
        throw new Error("hta/manifest-malformed: invalid host-call");
      }
      hostCalls[service] = Object.freeze([...methods]);
    }
  }
  return Object.freeze({
    namespace,identity,provider,module,abi,browserTarget,assets:Object.freeze(assets),
    handleTags:Object.freeze(handleTags),exports:Object.freeze(exports),
    hostCalls:Object.freeze(hostCalls)
  });
}

/** Strict data-only EDN subset shared by extension and package manifests. */
export function parseEdnData(source, errorCode = "edn/data-malformed") {
  const reader = new ManifestReader(source, errorCode);
  const value = reader.value();
  reader.space();
  if (reader.cursor !== source.length) throw new Error(`${errorCode}: trailing input`);
  return value;
}

function validPackagePath(value,suffix) {
  return typeof value === "string" && value.length>0 && !value.startsWith("/") && !value.split("/").includes("..") && !value.includes(":") && (!suffix || value.endsWith(suffix));
}

export async function loadHtaExtension({worker,descriptor,descriptorUrl,packageUrl,moduleBytes,hostCalls={}}) {
  if (descriptor === undefined) {
    if (!descriptorUrl) throw new Error("hta/manifest-missing: descriptor or descriptorUrl is required");
    const response = await fetch(descriptorUrl);
    if (!response.ok) throw new Error(`hta/manifest-load-failed: ${response.status}`);
    descriptor = await response.text();
  }
  const manifest = parseHtaManifest(descriptor);
  const base = packageUrl ?? descriptorUrl;
  let moduleUrl;
  let libraryUrl;
  if (manifest.provider === "wasm" && moduleBytes === undefined) {
    if (!base) throw new Error("hta/manifest-missing: packageUrl is required with inline descriptors");
    moduleUrl = new URL(manifest.module,base).toString();
  }
  if (manifest.provider === "wasm") {
    const library = manifest.assets.find(asset => asset.endsWith(".wasm") && asset !== manifest.module);
    if (library) {
      if (!base) throw new Error("hta/manifest-missing: packageUrl is required with inline descriptors");
      libraryUrl = new URL(library,base).toString();
    }
  }
  if (manifest.provider === "hta" && !worker) {
    if (!base) throw new Error("hta/manifest-missing: packageUrl is required with inline descriptors");
    worker = new Worker(new URL(manifest.browserTarget.module,base),{type:"module",name:`hara-${manifest.namespace}`});
  }
  if (!worker) throw new Error("hta/worker-missing: worker is required for WASM providers");
  const context = new HtaContext({
    worker,moduleUrl,moduleBytes,libraryUrl,hostCalls,handleTags:manifest.handleTags,
    manifest
  });
  return context;
}

function manifestField(map,name) { for (const [key,value] of map) if (key instanceof HtaKeyword && key.name===name) return value; }

class ManifestReader {
  constructor(source,errorCode="hta/manifest-malformed"){this.source=source;this.cursor=0;this.errorCode=errorCode;}
  error(message){return new Error(`${this.errorCode}: ${message}`);}
  space(){while(this.cursor<this.source.length){const ch=this.source[this.cursor];if(/[\s,]/.test(ch)){this.cursor++;continue;}if(ch===';'){while(this.cursor<this.source.length&&this.source[this.cursor]!=='\n')this.cursor++;continue;}break;}}
  value(){this.space();const ch=this.source[this.cursor++];if(ch===undefined)throw this.error("unexpected EOF");if(ch==='{')return this.map();if(ch==='[')return this.vector();if(ch==='\"')return this.string();if(ch===':')return new HtaKeyword(this.token());this.cursor--;const token=this.token();if(token==='nil')return null;if(token==='true')return true;if(token==='false')return false;if(/^-?[0-9]+$/.test(token))return Number(token);return new HtaSymbol(token);}
  map(){const result=new Map(),keys=new Set();for(;;){this.space();if(this.source[this.cursor]==='}'){this.cursor++;return result;}const key=this.value(),identity=displayHta(key);if(keys.has(identity))throw this.error("duplicate map key");keys.add(identity);this.space();if(this.source[this.cursor]==='}')throw this.error("map value missing");result.set(key,this.value());}}
  vector(){const result=[];for(;;){this.space();if(this.source[this.cursor]===']'){this.cursor++;return result;}result.push(this.value());}}
  string(){let result='';while(this.cursor<this.source.length){const ch=this.source[this.cursor++];if(ch==='\"')return result;if(ch==='\\'){const escaped=this.source[this.cursor++];if(escaped==='u'){const code=this.source.slice(this.cursor,this.cursor+4);if(!/^[0-9a-fA-F]{4}$/.test(code))throw this.error("invalid unicode escape");result+=String.fromCharCode(parseInt(code,16));this.cursor+=4;}else{const escapes={n:'\n',r:'\r',t:'\t',b:'\b',f:'\f','\"':'\"','\\':'\\'};if(!(escaped in escapes))throw this.error("invalid string escape");result+=escapes[escaped];}}else result+=ch;}throw this.error("unterminated string");}
  token(){this.space();const start=this.cursor;while(this.cursor<this.source.length&&!/[\s,{}\[\]\"]/ .test(this.source[this.cursor]))this.cursor++;if(start===this.cursor)throw this.error("invalid token");return this.source.slice(start,this.cursor);}
}

function writeValue(output, value, depth=0) {
  if(depth>HTA_MAX_NESTING_DEPTH)throw new Error("hta/value-too-deep: nesting exceeds 256");
  if (value === null || value === undefined) output.push(TAG.nil);
  else if (value === false) output.push(TAG.false);
  else if (value === true) output.push(TAG.true);
  else if (typeof value === "bigint") {
  if (value >= -(1n << 63n) && value < (1n << 63n)) {
    output.push(TAG.i64); writeI64(output, value);
  } else {
    output.push(TAG.bigInteger); writeBytes(output, encoder.encode(value.toString()));
  }
  } else if (Number.isSafeInteger(value) && !Object.is(value, -0)) {
  output.push(TAG.i64); writeI64(output, BigInt(value));
  } else if (typeof value === "number") {
    output.push(TAG.f64); writeF64(output, value);
  } else if (typeof value === "string") { output.push(TAG.string); writeBytes(output, encoder.encode(value)); }
  else if (value instanceof Uint8Array) { output.push(TAG.bytes); writeBytes(output, value); }
  else if (value instanceof HtaKeyword) { output.push(TAG.keyword); writeBytes(output, encoder.encode(value.name)); }
  else if (value instanceof HtaSymbol) { output.push(TAG.symbol); writeBytes(output, encoder.encode(value.name)); }
  else if (value instanceof HtaVar) { output.push(TAG.var); writeValue(output,value.symbol,depth+1); writeValue(output,value.value,depth+1); }
  else if (value instanceof HtaAtom) { output.push(TAG.atom); writeValue(output,value.value,depth+1); }
  else if (value instanceof HtaArray) { output.push(TAG.array); writeSequence(output,value.values,depth); }
  else if (value instanceof HtaObject) { output.push(TAG.object); writeU32(output,value.entries.length);for(const [key,item] of value.entries){writeValue(output,key,depth+1);writeValue(output,item,depth+1);} }
  else if (value instanceof HtaTuple) { output.push(TAG.tuple); writeSequence(output,value.values,depth); }
  else if (value instanceof HtaCons) { output.push(TAG.cons); writeSequence(output,value.values,depth); }
  else if (value instanceof HtaQueue) { output.push(TAG.queue); writeSequence(output,value.values,depth); }
  else if (value instanceof HtaDeque) { output.push(TAG.deque); writeSequence(output,value.values,depth); }
  else if (value instanceof HtaOrderedSet) { output.push(TAG.orderedSet); writeSequence(output,value.values,depth); }
  else if (value instanceof HtaSortedSet) { output.push(TAG.sortedSet); writeSequence(output,value.values,depth); }
  else if (value instanceof HtaOrderedMap || value instanceof HtaSortedMap || value instanceof HtaTrie || value instanceof HtaPriorityMap) { output.push(value instanceof HtaOrderedMap?TAG.orderedMap:value instanceof HtaSortedMap?TAG.sortedMap:value instanceof HtaTrie?TAG.trie:TAG.priorityMap);writeU32(output,value.entries.length);for(const [key,item] of value.entries){writeValue(output,key,depth+1);writeValue(output,item,depth+1);} }
  else if (value instanceof HtaTagged) { output.push(TAG.tagged);writeValue(output,value.tag,depth+1);writeValue(output,value.value,depth+1); }
  else if (value instanceof HtaExceptionInfo) { output.push(TAG.exceptionInfo);writeValue(output,value.message,depth+1);writeValue(output,value.data,depth+1);writeValue(output,value.cause,depth+1); }
  else if (value instanceof HtaStruct) { output.push(TAG.struct);writeValue(output,value.name,depth+1);writeValue(output,value.fields,depth+1);writeValue(output,value.values,depth+1); }
  else if (value instanceof HtaHandle) { if(value.released)throw new Error("hta/handle-released");output.push(TAG.handle);writeBytes(output,encoder.encode(value.owner));writeBytes(output,encoder.encode(value.type));writeI64(output,value.id); }
  else if (Array.isArray(value)) { output.push(TAG.vector); writeSequence(output, value, depth); }
  else if (value instanceof Set) { output.push(TAG.set); writeCanonical(output, [...value], depth); }
  else if (value instanceof Map) {
    const entries = [...value].map(([key, item]) => [bare(key,depth+1), bare(item,depth+1)]).sort((a, b) => compare(a[0], b[0]));
    output.push(TAG.map); writeU32(output, entries.length);
    for (const [key, item] of entries) { appendBytes(output,key); appendBytes(output,item); }
  } else throw new Error(`hta/value-unsupported: ${Object.prototype.toString.call(value)}`);
}

function bare(value,depth) { const output = []; writeValue(output, value, depth); return output; }
function writeSequence(output, values, depth) { writeU32(output, values.length); for (const value of values) writeValue(output, value, depth+1); }
function writeCanonical(output, values, depth) { const encoded = values.map(value=>bare(value,depth+1)).sort(compare); writeU32(output, encoded.length); for (const value of encoded) appendBytes(output,value); }
function compare(left, right) { for (let i=0;i<Math.min(left.length,right.length);i++) if(left[i]!==right[i]) return left[i]-right[i]; return left.length-right.length; }
function writeBytes(output, bytes) { writeU32(output, bytes.length); appendBytes(output,bytes); }
function appendBytes(output,bytes){if(output.length+bytes.length>HTA_MAX_FRAME_BYTES)throw new Error("hta/value-too-large: frame exceeds 64 MiB");for(let offset=0;offset<bytes.length;offset++)output.push(bytes[offset]);}
function writeU32(output, value) { if(value<0||value>0xffff_ffff)throw new Error("hta/value-too-large"); output.push(value>>>24,(value>>>16)&255,(value>>>8)&255,value&255); }
function writeI64(output, value) { const normalized=BigInt.asUintN(64,value); for(let shift=56n;shift>=0n;shift-=8n)output.push(Number((normalized>>shift)&255n)); }
function writeF64(output, value) { const bytes=new Uint8Array(8);new DataView(bytes.buffer).setFloat64(0,value,false);output.push(...bytes); }
function canonicalInteger(value) {
  return value>=BigInt(Number.MIN_SAFE_INTEGER)&&value<=BigInt(Number.MAX_SAFE_INTEGER)?Number(value):value;
}

class Reader {
  constructor(bytes, cursor) { this.bytes=bytes; this.cursor=cursor; }
  take(size) { const end=this.cursor+size; if(end>this.bytes.length)throw new Error("hta/value-malformed: truncated value"); const value=this.bytes.subarray(this.cursor,end);this.cursor=end;return value; }
  u32() { const value=this.take(4); return ((value[0]*0x1000000)+(value[1]<<16)+(value[2]<<8)+value[3])>>>0; }
  data() { return this.take(this.u32()); }
  sequence(depth) { const size=this.u32();if(size>this.bytes.length-this.cursor)throw new Error("hta/value-malformed: impossible sequence length");const result=[]; for(let i=0;i<size;i++)result.push(this.value(depth+1)); return result; }
  value(depth=0) {
    if(depth>HTA_MAX_NESTING_DEPTH)throw new Error("hta/value-too-deep: nesting exceeds 256");
    const tag=this.take(1)[0];
    if(tag===TAG.nil)return null;if(tag===TAG.false)return false;if(tag===TAG.true)return true;
    if(tag===TAG.i64){const bytes=this.take(8);let value=0n;for(const byte of bytes)value=(value<<8n)|BigInt(byte);value=BigInt.asIntN(64,value);return canonicalInteger(value);}
    if(tag===TAG.bigInteger){const text=decoder.decode(this.data());if(!/^-?(0|[1-9][0-9]*)$/.test(text)||text==="-0")throw new Error("hta/value-malformed: invalid big integer");return canonicalInteger(BigInt(text));}
    if(tag===TAG.f64){const bytes=this.take(8);return new DataView(bytes.buffer,bytes.byteOffset,8).getFloat64(0,false);}
    if(tag===TAG.string)return decoder.decode(this.data());if(tag===TAG.bytes)return this.data().slice();
    if(tag===TAG.keyword)return new HtaKeyword(decoder.decode(this.data()));if(tag===TAG.symbol)return new HtaSymbol(decoder.decode(this.data()));
    if(tag===TAG.list||tag===TAG.vector)return this.sequence(depth);if(tag===TAG.set)return new Set(this.sequence(depth));
    if(tag===TAG.tuple)return new HtaTuple(this.sequence(depth));if(tag===TAG.cons)return new HtaCons(this.sequence(depth));if(tag===TAG.queue)return new HtaQueue(this.sequence(depth));if(tag===TAG.deque)return new HtaDeque(this.sequence(depth));
    if(tag===TAG.orderedSet)return new HtaOrderedSet(this.sequence(depth));if(tag===TAG.sortedSet)return new HtaSortedSet(this.sequence(depth));
    if(tag===TAG.map){const size=this.u32();if(size>(this.bytes.length-this.cursor)/2)throw new Error("hta/value-malformed: impossible map length");const result=new Map();for(let i=0;i<size;i++)result.set(this.value(depth+1),this.value(depth+1));return result;}
    if(tag===TAG.orderedMap||tag===TAG.sortedMap||tag===TAG.trie||tag===TAG.priorityMap){const size=this.u32();if(size>(this.bytes.length-this.cursor)/2)throw new Error("hta/value-malformed: impossible map length");const entries=[];for(let i=0;i<size;i++)entries.push([this.value(depth+1),this.value(depth+1)]);return tag===TAG.orderedMap?new HtaOrderedMap(entries):tag===TAG.sortedMap?new HtaSortedMap(entries):tag===TAG.trie?new HtaTrie(entries):new HtaPriorityMap(entries);}
    if(tag===TAG.var){const symbol=this.value(depth+1),value=this.value(depth+1);if(!(symbol instanceof HtaSymbol))throw new Error("hta/value-malformed: invalid var symbol");return new HtaVar(symbol,value);}
    if(tag===TAG.atom)return new HtaAtom(this.value(depth+1));
    if(tag===TAG.array)return new HtaArray(this.sequence(depth));
    if(tag===TAG.object){const size=this.u32();if(size>(this.bytes.length-this.cursor)/2)throw new Error("hta/value-malformed: impossible object length");const entries=[];for(let i=0;i<size;i++){const key=this.value(depth+1);if(typeof key!=="string")throw new Error("hta/value-malformed: invalid object key");entries.push([key,this.value(depth+1)]);}return new HtaObject(entries);}
    if(tag===TAG.handle){const owner=decoder.decode(this.data()),type=decoder.decode(this.data()),bytes=this.take(8);let id=0n;for(const byte of bytes)id=(id<<8n)|BigInt(byte);return new HtaHandle(owner,type,id);}
    if(tag===TAG.tagged){const tagValue=this.value(depth+1);if(!(tagValue instanceof HtaSymbol))throw new Error("hta/value-malformed: invalid tagged literal tag");return new HtaTagged(tagValue,this.value(depth+1));}
    if(tag===TAG.exceptionInfo){const message=this.value(depth+1);if(typeof message!=="string")throw new Error("hta/value-malformed: invalid exception message");return new HtaExceptionInfo(message,this.value(depth+1),this.value(depth+1));}
    if(tag===TAG.struct){const name=this.value(depth+1),fields=this.value(depth+1),values=this.value(depth+1);if(typeof name!=="string"||!Array.isArray(fields)||!fields.every(field=>typeof field==="string")||!Array.isArray(values)||fields.length!==values.length)throw new Error("hta/value-malformed: invalid struct");return new HtaStruct(name,fields,values);}
    throw new Error("hta/value-malformed: unknown value tag");
  }
}

export class HtaContext {
  constructor({ worker, moduleUrl, moduleBytes, libraryUrl, libraryBytes, hostCalls = {}, filesystemHost = hostCalls.filesystemHost ?? null, handleTags = {}, promiseProvider = new BrowserPromiseProvider(), kernelId = null, manifest = null }) {
    this.worker=worker;this.hostCalls=hostCalls;this.filesystemHost=filesystemHost;this.handleTags=handleTags;this.promiseProvider=promiseProvider;this.kernelId=kernelId;this.manifest=manifest;this.allowedExports=manifest ? new Set(manifest.exports) : null;this.allowedHostCalls=manifest ? new Set(Object.entries(manifest.hostCalls).flatMap(([service,methods])=>methods.map(method=>`${service}/${method}`))) : null;this.next=1;this.pending=new Map();this.sessions=new Map();this.mounts=new Set();this.closed=false;
    this.ready=new Promise((resolve,reject)=>{this.readyResolve=resolve;this.readyReject=reject;});
    this.ready.catch(()=>{});
    worker.addEventListener("message", event=>this.message(event.data));
    worker.addEventListener("error", error=>this.fail(error));
    worker.postMessage({type:"init",moduleUrl,moduleBytes,libraryUrl,libraryBytes});
  }
  call(target, args=[]) { let id=null,cancelled=false;
    return this.promiseProvider.create((resolve,reject,onCancel)=>{
      onCancel(()=>{cancelled=true;if(id!==null)this.worker.postMessage({type:"cancel",id});});
      this.ready.then(()=>{if(cancelled)return;if(this.closed)throw new Error("hta/context-closed");if(this.allowedExports && !this.allowedExports.has(target))throw new Error(`hta/export-denied: ${target}`);validateHandles(args,this);id=this.next++;this.pending.set(id,{resolve,reject});this.worker.postMessage({type:"call",id,frame:encodeHta([target,args])});}).catch(reject);
    });
  }
  releaseHandle(handle){if(handle.context!==this)throw new Error("hta/handle-owner-mismatch");if(this.closed)return;const wireHandle=new HtaHandle(handle.owner,handle.type,handle.id);this.worker.postMessage({type:"release",frame:encodeHta(wireHandle)});}
  async createSession(name){await this.call("session/create",[name]);return this.session(name);}
  async createFilesystem(descriptor={provider:"memory"}){
    if(!descriptor||typeof descriptor!=="object")throw new Error("filesystem/descriptor-invalid");
    if(!this.filesystemHost)throw new Error("filesystem/host-unavailable");
    const wire=new Map(Object.entries(descriptor));
    const mountId=await this.call("filesystem/create",[wire]);
    try{await this.filesystemHost.register(this,mountId,descriptor);this.mounts.add(mountId);return mountId;}
    catch(error){await this.call("filesystem/close",[mountId]).catch(()=>{});throw error;}
  }
  async filesystemInfo(mountId){return this.call("filesystem/info",[mountId]);}
  async closeFilesystem(mountId){const result=await this.call("filesystem/close",[mountId]);await this.filesystemHost.close(this,mountId);this.mounts.delete(mountId);return result;}
  session(name="ROOT"){let session=this.sessions.get(name);if(!session){session=new HtaSession(this,name);this.sessions.set(name,session);}return session;}
  listSessions(){return this.call("session/list",[]);}
  async message(message) {
    if(message.type==="ready"){this.readyResolve();return;}if(message.type==="fatal"){this.fail(new Error(message.error?.message??"HTA worker failed"));return;}
    if(message.type==="result"){const pending=this.pending.get(message.id);if(!pending)return;this.pending.delete(message.id);const value=bindHandles(decodeHta(message.frame),this);message.ok?pending.resolve(value):pending.reject(errorFrom(value));return;}
    if(message.type==="host-call"){const key=`${message.service}/${message.method}`,handler=this.hostCalls[key],sessionId=message.session??"ROOT";try{if(this.allowedHostCalls && !this.allowedHostCalls.has(key))throw new Error(`hta/host-call-denied: ${key}`);if(!handler)throw new Error(`hta/host-call-denied: ${key}`);const value=await handler.call({context:this.session(sessionId),kernelContext:this,kernelId:this.kernelId??null,sessionId,mountId:message.mount??null,task:message.task},...decodeHta(message.frame));this.worker.postMessage({type:"delivery",call:message.call,ok:true,frame:encodeHta(value)});}catch(error){this.worker.postMessage({type:"delivery",call:message.call,ok:false,frame:encodeHta(errorValue(error))});}}
  }
  fail(error){if(this.closed)return;this.closed=true;this.readyReject(error);for(const pending of this.pending.values())pending.reject(error);this.pending.clear();}
  close(){if(!this.closed){this.closed=true;const error=new Error("hta/context-closed");this.readyReject(error);for(const pending of this.pending.values())pending.reject(error);this.pending.clear();}const closes=[...this.mounts].map(mountId=>this.filesystemHost?.close(this,mountId).catch(()=>{}));this.mounts.clear();this.worker.postMessage({type:"close"});this.worker.terminate();return Promise.all(closes);}
}

export class HtaSession {
  constructor(context,name){if(typeof name!=="string"||!name.length)throw new Error("INVALID_SESSION_NAME");this.context=context;this.name=name;}
  call(target,args=[]){if(target==="eval")return this.eval(args[0]);if(target==="eval-vm")return this.evalVm(args[0]);if(target==="eval-bound")return this.evalBound(args[0],args[1]);if(target==="complete")return this.complete(args[0]);return this.context.call(target,args);}
  eval(source){return this.context.call("session/eval",[this.name,source]);}
  evalVm(source){return this.context.call("session/eval-vm",[this.name,source]);}
  prepareVm(source){return this.context.call("session/prepare-vm",[this.name,source]);}
  invokeVm(program){return this.context.call("session/invoke-vm",[this.name,program]);}
  evalBound(source,bindings=[]){return this.context.call("session/eval-bound",[this.name,source,bindings]);}
  complete(prefix){return this.context.call("session/complete",[this.name,prefix]);}
  info(){return this.context.call("session/info",[this.name]);}
  async attachFilesystem(mountId){return this.context.call("session/attach-filesystem",[this.name,mountId]);}
  async detachFilesystem(){return this.context.call("session/detach-filesystem",[this.name]);}
  async close(){const result=await this.context.call("session/close",[this.name]);this.context.sessions.delete(this.name);return result;}
}

function bindHandles(value,context){if(value instanceof HtaHandle){value.context=context;const tag=context.handleTags[value.type];if(tag){value.displayTag=tag;value.displayKind=value.type;}return value;}if(Array.isArray(value)){value.forEach(item=>bindHandles(item,context));}else if(value instanceof Set){for(const item of value)bindHandles(item,context);}else if(value instanceof Map){for(const [key,item]of value){bindHandles(key,context);bindHandles(item,context);}}return value;}
function validateHandles(value,context){if(value instanceof HtaHandle){if(value.released)throw new Error("hta/handle-released");if(value.context!==context)throw new Error("hta/handle-owner-mismatch");return;}if(Array.isArray(value)){value.forEach(item=>validateHandles(item,context));}else if(value instanceof Set){for(const item of value)validateHandles(item,context);}else if(value instanceof Map){for(const [key,item]of value){validateHandles(key,context);validateHandles(item,context);}}}
function errorValue(error){return new Map([[new HtaKeyword("code"),new HtaKeyword("host/error")],[new HtaKeyword("message"),String(error?.message??error)],[new HtaKeyword("origin"),new HtaKeyword("browser")],[new HtaKeyword("retryable"),false]]);}
function errorFrom(value){if(value instanceof Error)return value;if(value instanceof Map){let message="HTA request failed",code;for(const[key,item]of value)if(key instanceof HtaKeyword&&key.name==="message")message=String(item);else if(key instanceof HtaKeyword&&key.name==="code")code=item instanceof HtaKeyword?item.name:String(item);const error=new Error(message);error.code=code;error.data=value;return error;}return new Error(String(value));}
