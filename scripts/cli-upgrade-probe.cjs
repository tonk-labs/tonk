// Runs an authentic extracted release in a disposable node:24-trixie-slim
// container. The explicit loader works around the release's existing Nix
// interpreter defect without changing the executable. No user profile touched.
// /probe/linux015/package/bin/tonk comes from @tonk/cli-linux-x64@0.6.15.
// Optional third argument targets the production Miniflare Worker fixture.
const http=require('http'),fs=require('fs'),assert=require('assert/strict'),{spawn}=require('child_process');
const version=process.argv[2]||'015';
const detail="This client uses an obsolete Tonk storage protocol. macOS arm64: npm install -g @tonk/cli@0.6.17 ; then run tonk --version, check PATH, and retry. Linux: a portable direct-UCAN CLI release is required; npm 0.6.17 still has the Nix-loader defect and is not a working upgrade. Release status: https://github.com/tonk-labs/tonk/pull/1085 . Do not use the frozen @stable alias. Browser users should reload the current Tonk app.";
const server=http.createServer((req,res)=>{
 console.log('REQUEST',req.method,new URL(req.url,'http://localhost').pathname,{direct:!!req.headers.authorization,contentType:req.headers['content-type']});
 req.resume();res.writeHead(426,{'Content-Type':'application/json'});res.end(JSON.stringify({kind:'Unclassified',detail}));
});
server.listen(0,'127.0.0.1',async()=>{
 const state='/tmp/tonk-version-probe';fs.mkdirSync(state,{recursive:true});
 const env={...process.env,TONK_SPACES_STATE:state,TONK_TELEMETRY_STATE:state,DO_NOT_TRACK:'1',TONK_NO_UPDATE_CHECK:'1',TONK_UPDATE_STATE:state,TONK_UNSAFE_ALLOW_DEVICE_ROOT:'1'};
 async function run(args){console.log('COMMAND',args.join(' '));return await new Promise(resolve=>{let output='';const child=spawn("/lib64/ld-linux-x86-64.so.2",[`/probe/linux${version}/package/bin/tonk`,...args],{cwd:state,env});for(const stream of [child.stdout,child.stderr])stream.on('data',d=>{output+=d;process.stdout.write(d)});child.on('close',code=>{console.log('EXIT',code);resolve({code,output})});});}

 await run(['--version']);await run(['space','new','demo','--site',state+'/site']);
 await run(['remote','add','origin',process.argv[3]||`http://127.0.0.1:${server.address().port}/ucan/`]);
 for(const cmd of [['pull'],['push'],['--verbose','pull']]) {
   const result=await run(cmd);
   if(version==='015') {
     assert.equal(result.code,4);assert.match(result.output,/npm install -g @tonk\/cli@0\.6\.17/);
     assert.match(result.output,/tonk --version/);assert.match(result.output,/Linux/);assert.match(result.output,/not a working upgrade/);assert.match(result.output,/https:\/\/github.com\/tonk-labs\/tonk\/pull\/1085/);
   }
 }
 console.log('Released CLI upgrade guidance assertions passed');
 server.close();
});
