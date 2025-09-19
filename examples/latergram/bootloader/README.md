# bootloader

a web bootloader taking inspiration from [trail runner](https://github.com/pvh/trail-runner)!

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

