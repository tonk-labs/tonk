# bootloader

how to set up:

1. start the tonk server:
```
cd server
tsx src/index.ts 8081 bundle.tonk
```
2. to replace the compiler, i have the script inside `/deploy` which reads from pre-compiled code. run this and upload to the tonk server: 
```
deno upload-files-to-tonk.js ./path/to/compiled/code
```
3. start the bootloader and access the file system:
```
npm run build && cp dist-sw/service-worker-bundled.js src/service-worker-bundled.js && vite dev
```

## things i have tested 
- weird mime types that are not specified by the code 
- routing using a url change 
- hard reloads (cmd+shift+r) 