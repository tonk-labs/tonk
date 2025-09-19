# bootloader

a web bootloader taking inspiration from [trail runner](https://github.com/pvh/trail-runner)!

## setup

1. start the tonk server:
```
cd server
tsx src/index.ts 8081 bundle.tonk
```
2. to replace the compiler, i have the script inside `/deploy` which reads from pre-compiled code. run this and upload to the tonk server: 
```
cd deploy
deno upload-files-to-tonk.js ./eg-app
```
3. start the bootloader and access the file system:
```
npm run build && cp dist-sw/service-worker-bundled.js src/service-worker-bundled.js && vite dev
```
> in deployment, remove the hardcoded manifest bytes at line 14 of `service-worker.js`. this is here for testing reasons only!

## design choices 

Below are the design choices I made and why:

### Rerouting to the site's index when a html file is not found
In cases where the user reloads inside a react single page application using virtual routes, the service worker tries to find a file corresponding to this virtual route, and obviously fails. I fix this by attempting to load `/app/index.html` in cases where we have failed to load another `.html` file. Importantly, the URL and cache remain untouched, so the app context is retained and the app remains at its current route.  

### Mime type identification 
I removed the reliance upon hardcoded file headers, instead using an imported library to infer based on the file path of the document for robustness. 