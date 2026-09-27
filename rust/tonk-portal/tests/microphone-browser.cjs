const { chromium } = require(process.env.TONK_PLAYWRIGHT_PATH || 'playwright');
const fs = require('fs');
const assert = require('assert/strict');
(async()=>{
 const browser=await chromium.launch({executablePath:process.env.TONK_CHROME_PATH || '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',headless:true,args:['--use-fake-device-for-media-stream','--use-fake-ui-for-media-stream']});
 try{
 const page=await browser.newPage();
 const errors=[];page.on('pageerror',e=>errors.push(e.message));

 const base=require('path').resolve(__dirname,'../src')+'/';
 const bootstrap=fs.readFileSync(base+'bootstrap.js','utf8');
 const mic=fs.readFileSync(base+'microphone.js','utf8').replaceAll('export function','function');
 const setup=`${mic}\nconst bound=new Map();window.addEventListener('message',e=>{const child=[...document.querySelectorAll('iframe')].find(f=>f.contentWindow===e.source);if(!child||e.data?.type!=='hello')return;const p=e.ports[0];bound.set(child,p);p.onmessage=e=>handleMicrophone(e.data,p);p.postMessage({type:'ready',context:{}})});window.clearChildren=()=>{for(const [f,p]of bound){disposeMicrophone(p);p.close();f.remove()}bound.clear()};`;
 const leaf=`<body><script>${bootstrap}</script><p>Recording guest</p>`;
 await page.route('**/__dev/mic-test',route=>route.fulfill({contentType:'text/html',body:'<!doctype html><html><body></body></html>'}));
 await page.goto('http://localhost:8091/__dev/mic-test');
 await page.addScriptTag({content:setup});
 await page.evaluate(html=>{const f=document.createElement('iframe');f.sandbox='allow-scripts';f.srcdoc=html;document.body.append(f)},`<body><script>${bootstrap}</script><script>${setup}</script>`);
 await page.waitForFunction(()=>document.querySelector('iframe'));
 let middle;for(let i=0;i<100;i++){middle=page.frames().find(f=>f.parentFrame()===page.mainFrame());if(middle&&await middle.evaluate(()=>!!window.clearChildren).catch(()=>false))break;await new Promise(r=>setTimeout(r,50))}
 await middle.evaluate(html=>{const f=document.createElement('iframe');f.sandbox='allow-scripts';f.srcdoc=html;document.body.append(f)},leaf);
 let guest;for(let i=0;i<100;i++){guest=page.frames().find(f=>f.parentFrame()?.parentFrame());if(guest&&await guest.evaluate(()=>!!window.tonk).catch(()=>false))break;await new Promise(r=>setTimeout(r,50))}
 assert(guest,'nested sandbox exists: '+JSON.stringify(errors));assert.equal(await guest.evaluate(()=>origin),'null');
 await page.evaluate(()=>{window.tracks=[];const get=navigator.mediaDevices.getUserMedia.bind(navigator.mediaDevices);navigator.mediaDevices.getUserMedia=async c=>{const s=await get(c);tracks.push(...s.getTracks());return s}});
 const begin=(method='recordAudio',options={})=>guest.evaluate(({method,options})=>{window.outcome=null;window.recording=null;window.levels=0;window.tonk[method]({maxDurationSeconds:3,...options,onLevel:()=>levels++}).then(s=>{window.recording=s;s.result.then(async b=>{
 const outcomeValue={bytes:b.size,type:b.type,levels};
 if(b.type.startsWith('video/')){const v=document.createElement('video');v.src=URL.createObjectURL(b);await new Promise((resolve,reject)=>{v.onloadedmetadata=resolve;v.onerror=reject});outcomeValue.width=v.videoWidth;outcomeValue.height=v.videoHeight;URL.revokeObjectURL(v.src)}
 outcome=outcomeValue;
 },e=>outcome={error:e.name})},e=>outcome={error:e.name})},{method,options});
 await begin();await page.getByRole('button',{name:'Allow recording',exact:true}).click();
 await guest.waitForFunction(()=>window.levels>=3);await guest.evaluate(()=>recording.stop());await guest.waitForFunction(()=>outcome);
 const result=await guest.evaluate(()=>outcome);assert(result.bytes>0);assert(result.levels>=3);assert.match(result.type,/audio/);
 assert(await page.evaluate(()=>tracks.every(t=>t.readyState==='ended')));
 const videos=[];
 for(const audio of [true,false]){
 await begin('recordVideo',{audio,maxDurationSeconds:1});
 await page.getByRole('heading',{name:audio?'Record camera and microphone?':'Record camera video?',exact:true}).waitFor();
 await page.getByRole('button',{name:'Allow recording',exact:true}).click();
 await guest.waitForFunction(()=>window.recording);
 const kinds=await page.evaluate(()=>tracks.filter(t=>t.readyState==='live').map(t=>t.kind).sort());assert.deepEqual(kinds,audio?['audio','video']:['video']);
 await guest.waitForFunction(()=>outcome);const video=await guest.evaluate(()=>outcome);assert(video.bytes>0);assert.match(video.type,/video/);assert(video.width>0&&video.height>0);videos.push(video);
 assert(await page.evaluate(()=>tracks.every(t=>t.readyState==='ended')));
 }
 await begin('recordVideo');await page.getByRole('button',{name:'Cancel',exact:true}).click();await guest.waitForFunction(()=>outcome);assert.equal((await guest.evaluate(()=>outcome)).error,'AbortError');
 await begin('recordVideo');await page.getByRole('button',{name:'Allow recording',exact:true}).click();await guest.waitForFunction(()=>window.recording);
 assert.equal(await guest.evaluate(()=>tonk.recordAudio().then(()=>null,e=>e.name)),'InvalidStateError');
 await guest.evaluate(()=>recording.cancel());await guest.waitForFunction(()=>outcome);assert.equal((await guest.evaluate(()=>outcome)).error,'AbortError');
 await page.waitForFunction(()=>tracks.every(t=>t.readyState==='ended'));
 await guest.evaluate(()=>{const c=new AbortController();c.abort();window.aborted=tonk.recordVideo({signal:c.signal}).then(()=>null,e=>e.name)});assert.equal(await guest.evaluate(()=>aborted),'AbortError');
 await begin();await page.getByRole('button',{name:'Cancel',exact:true}).click();await guest.waitForFunction(()=>outcome);assert.equal((await guest.evaluate(()=>outcome)).error,'AbortError');
 await page.evaluate(()=>{window.savedGetUserMedia=navigator.mediaDevices.getUserMedia;navigator.mediaDevices.getUserMedia=async()=>{throw new DOMException('Permission denied','NotAllowedError')}});
 await begin('recordVideo');await page.getByRole('button',{name:'Allow recording',exact:true}).click();await guest.waitForFunction(()=>outcome);assert.equal((await guest.evaluate(()=>outcome)).error,'NotAllowedError');
 await page.evaluate(()=>{navigator.mediaDevices.getUserMedia=window.savedGetUserMedia});
 await begin('recordVideo');await page.getByRole('button',{name:'Allow recording',exact:true}).click();await guest.waitForFunction(()=>window.levels>=3);
 await page.getByRole('button',{name:'Stop',exact:true}).click();await guest.waitForFunction(()=>outcome);assert((await guest.evaluate(()=>outcome)).bytes>0);
 assert(await page.evaluate(()=>tracks.every(t=>t.readyState==='ended')));
 await begin('recordVideo');await page.getByRole('button',{name:'Allow recording',exact:true}).click();await guest.waitForFunction(()=>window.levels>=2);await page.evaluate(()=>clearChildren());
 await page.waitForFunction(()=>tracks.every(t=>t.readyState==='ended'));
 assert.deepEqual(errors,[]);
 console.log(JSON.stringify({nestedOpaqueFrames:2,recording:result,videos,cancel:'passed',teardownStopsTracks:'passed',pageErrors:errors}));
 } finally {await browser.close()}
})().catch(e=>{console.error(e);process.exit(1)});
