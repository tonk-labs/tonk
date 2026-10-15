self.addEventListener('install',e=>e.waitUntil(self.skipWaiting()));
self.addEventListener('activate',e=>e.waitUntil(self.clients.claim()));
self.addEventListener('fetch',e=>{
 const url=new URL(e.request.url);if(!url.pathname.endsWith('/stream'))return;
 const mode=url.searchParams.get('mode'),enc=new TextEncoder();let n=0;
 const parts=['data: snapshot\n\n','data: checkpoint\n\n'];
 const source=mode==='pull0' ? {pull(c){if(n<2)c.enqueue(enc.encode(parts[n++]));}} : {start(c){if(mode==='combined')c.enqueue(enc.encode(parts.join('')));else parts.forEach(p=>c.enqueue(enc.encode(p)));}};
 e.respondWith(new Response(new ReadableStream(source,mode==='pull0'?{highWaterMark:0}:undefined),{headers:{'content-type':'text/event-stream'}}));
});
