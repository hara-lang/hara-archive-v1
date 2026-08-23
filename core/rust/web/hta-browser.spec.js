import { test, expect } from "@playwright/test";
test("real worker resumes a pending HTA evaluator fiber",async({page})=>{await page.goto("/rust/web/hta-browser.html");await expect.poll(()=>page.evaluate(()=>window.htaSmoke?.then(String))).toBe("42");await page.evaluate(()=>window.htaContext.close());});

test("browser promise provider follows the real Chromium event loop",async({page})=>{
  await page.goto("/rust/web/hta-browser.html");
  const result=await page.evaluate(async()=>{
    const {BrowserPromiseProvider}=await import("/rust/web/hta.js");
    const provider=new BrowserPromiseProvider(),events=[];
    const source=provider.run(()=>{events.push("run");return 21;});
    const chained=provider.then(source,value=>{events.push("then");return value*2;});
    events.push("sync");
    return {value:await chained,events,native:chained instanceof Promise};
  });
  expect(result).toEqual({value:42,events:["sync","run","then"],native:true});
});

test("fresh browser sandbox evaluates without native authority",async({page})=>{
  await page.goto("/rust/web/hta-browser.html");
  const result=await page.evaluate(async()=>{
    const bytes=new Uint8Array(await (await fetch("/rust/raw/target/wasm32-unknown-unknown/browser-release/hara-wasm-core.wasm")).arrayBuffer());
    const {BrowserWasmSandbox}=await import("/rust/web/packages/hta/sandbox.js");
    const create=()=>new BrowserWasmSandbox({
      workerUrl:new URL("/rust/web/hta-worker.js",location.href),
      moduleBytes:bytes,
    });
    const sandbox=create();
    const completed=await sandbox.run({operation:"sandbox.eval",source:"(+ 40 2)"});
    let secondRun;
    try{await sandbox.run({operation:"sandbox.eval",source:"(+ 1 1)"});}
    catch(error){secondRun=error.code;}
    const isolated=await create().run({
      operation:"sandbox.eval",
      source:"(and (nil? (resolve 'Runtime)) (nil? (resolve 'std.native.Runtime/call)) (nil? (resolve 'Host/call)) (nil? (resolve 'File/read)))",
    });
    return {completed,secondRun,isolated};
  });
  expect(result.completed.value).toEqual({text:"42",json:42});
  expect(result.completed.cleanup).toBe("completed");
  expect(result.secondRun).toBe("sandbox/not-reusable");
  expect(result.isolated.value).toEqual({text:"true",json:true});
});
