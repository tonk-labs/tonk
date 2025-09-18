# bootloader

here is my current workflow:

1. start a server on port 8081 holding a tonk bundle matching the bytes included in the file
    - i know this is not ideal at all, i just need to figure out how to actually load the bytes correctly each time. please feel free to swap this out
2. to replace the compiler, i have the script inside `/deploy` which reads from pre-compiled code. to run this and upload to the tonk server, run `deno upload-files-to-tonk.js ./path/to/compiled/code`.
3. to start the bootloader and access the file system, run the script below (i should really make this a bash script!)
```
npm run build && cp dist-sw/service-worker-bundled.js src/service-worker-bundled.js && vite dev
```

## things i have tested 
- weird mime types that are not specified by the code 
- routing using a url change 
- hard reloads (cmd+shift+r) 