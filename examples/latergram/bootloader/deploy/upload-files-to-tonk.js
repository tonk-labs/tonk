import { TonkCore, initializeTonk } from '../src/tonk/index-slim.js';
import * as fs from 'node:fs';
import * as path from 'node:path';

const manifestData = new Uint8Array([80, 75, 3, 4, 20, 0, 0, 0, 8, 0, 0, 0, 33, 0, 189, 99, 1, 115, 217, 0, 0, 0, 58, 1, 0, 0, 13, 0, 0, 0, 109, 97, 110, 105, 102, 101, 115, 116, 46, 106, 115, 111, 110, 77, 143, 49, 79, 195, 48, 16, 133, 247, 252, 138, 147, 87, 72, 228, 52, 137, 160, 222, 88, 144, 24, 138, 58, 52, 161, 8, 49, 84, 245, 33, 76, 154, 187, 234, 236, 134, 160, 42, 255, 29, 217, 64, 197, 248, 125, 126, 79, 126, 119, 206, 0, 212, 176, 35, 247, 134, 62, 116, 40, 222, 49, 41, 3, 229, 117, 244, 227, 133, 207, 25, 64, 10, 126, 176, 252, 61, 3, 168, 193, 81, 98, 157, 1, 204, 169, 34, 204, 225, 193, 42, 3, 202, 251, 119, 172, 199, 250, 185, 181, 237, 106, 187, 222, 158, 6, 234, 187, 169, 92, 57, 251, 180, 86, 41, 138, 20, 228, 235, 200, 142, 130, 87, 6, 94, 94, 147, 36, 12, 159, 44, 125, 43, 238, 159, 156, 30, 57, 96, 100, 58, 29, 14, 63, 166, 67, 178, 233, 235, 223, 101, 211, 134, 169, 191, 32, 128, 218, 11, 238, 2, 218, 187, 16, 183, 44, 244, 162, 201, 245, 50, 47, 111, 54, 101, 99, 154, 91, 83, 233, 162, 174, 170, 43, 173, 141, 214, 105, 77, 234, 224, 116, 100, 9, 104, 239, 133, 135, 88, 11, 76, 125, 190, 103, 65, 24, 117, 81, 22, 90, 165, 224, 28, 143, 205, 230, 111, 80, 75, 3, 4, 20, 0, 0, 0, 8, 0, 0, 0, 33, 0, 68, 12, 55, 184, 214, 0, 0, 0, 213, 0, 0, 0, 59, 0, 0, 0, 115, 116, 111, 114, 97, 103, 101, 47, 115, 115, 47, 104, 101, 52, 118, 52, 89, 85, 100, 85, 77, 88, 80, 88, 117, 109, 110, 107, 86, 120, 49, 77, 105, 100, 87, 80, 47, 115, 110, 97, 112, 115, 104, 111, 116, 47, 98, 117, 110, 100, 108, 101, 95, 101, 120, 112, 111, 114, 116, 107, 205, 247, 106, 182, 126, 42, 234, 204, 112, 138, 145, 81, 160, 187, 51, 224, 202, 92, 43, 247, 132, 29, 11, 62, 134, 207, 190, 34, 116, 149, 177, 187, 236, 199, 223, 222, 178, 72, 131, 63, 91, 111, 27, 68, 148, 221, 96, 14, 57, 236, 253, 97, 87, 153, 255, 251, 217, 18, 57, 231, 150, 77, 250, 35, 196, 206, 200, 196, 204, 36, 204, 172, 204, 228, 192, 236, 204, 20, 198, 196, 197, 200, 194, 196, 34, 106, 160, 200, 164, 204, 110, 194, 232, 196, 22, 198, 30, 46, 208, 192, 200, 196, 196, 192, 196, 88, 199, 192, 198, 196, 80, 199, 192, 88, 207, 192, 196, 206, 192, 194, 196, 192, 192, 194, 196, 92, 197, 145, 156, 145, 153, 147, 82, 148, 154, 199, 146, 151, 152, 155, 202, 85, 146, 153, 155, 90, 92, 146, 152, 91, 80, 204, 82, 82, 89, 144, 202, 158, 92, 148, 154, 88, 146, 154, 194, 145, 155, 159, 146, 153, 150, 153, 154, 194, 198, 80, 197, 86, 195, 88, 199, 204, 200, 86, 203, 196, 200, 192, 204, 88, 195, 32, 198, 96, 198, 148, 162, 159, 146, 89, 52, 127, 203, 140, 195, 83, 141, 33, 36, 27, 3, 35, 0, 80, 75, 1, 2, 20, 3, 20, 0, 0, 0, 8, 0, 0, 0, 33, 0, 189, 99, 1, 115, 217, 0, 0, 0, 58, 1, 0, 0, 13, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 164, 129, 0, 0, 0, 0, 109, 97, 110, 105, 102, 101, 115, 116, 46, 106, 115, 111, 110, 80, 75, 1, 2, 20, 3, 20, 0, 0, 0, 8, 0, 0, 0, 33, 0, 68, 12, 55, 184, 214, 0, 0, 0, 213, 0, 0, 0, 59, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 164, 129, 4, 1, 0, 0, 115, 116, 111, 114, 97, 103, 101, 47, 115, 115, 47, 104, 101, 52, 118, 52, 89, 85, 100, 85, 77, 88, 80, 88, 117, 109, 110, 107, 86, 120, 49, 77, 105, 100, 87, 80, 47, 115, 110, 97, 112, 115, 104, 111, 116, 47, 98, 117, 110, 100, 108, 101, 95, 101, 120, 112, 111, 114, 116, 80, 75, 5, 6, 0, 0, 0, 0, 2, 0, 2, 0, 164, 0, 0, 0, 51, 2, 0, 0, 0, 0]);

let tonk;

async function loadDistFiles(distPath, basePath = '') {
    const files = fs.readdirSync(distPath);

    for (const file of files) {
        const fullPath = path.join(distPath, file);
        const relativePath = basePath ? `${basePath}/${file}` : file;

        if (fs.statSync(fullPath).isDirectory()) {
          await tonk.createDirectory(relativePath);
          loadDistFiles(fullPath, relativePath);
        } else {
          const content = fs.readFileSync(fullPath, 'utf-8');
          console.log(relativePath);
          await tonk.createFile(relativePath, content);
          console.log(await tonk.readFile(relativePath));
        }
    }
} 


async function uploadFilesToTonk() {
  try {
    // 1. Initialize Tonk WASM
    //await initializeTonk('http://localhost:8081/tonk_core_bg.wasm');

    await initializeTonk({
      wasmPath: 'http://localhost:8081/tonk_core_bg.wasm'
    });

    // 2. Fetch and load the manifest
    const manifestResponse = await fetch('http://localhost:8081/.manifest.tonk');
    const manifestBytes = await manifestResponse.arrayBuffer();

    // 3. Create Tonk instance from bundle
    tonk = await TonkCore.fromBytes(manifestData);

    // 4. Connect to the server via WebSocket
    await tonk.connectWebsocket('ws://localhost:8081');

    console.log('Connected to Tonk server');

    try {
      await tonk.exists('/app');
      console.log("this exists");
    }
    catch (error) {
      console.log(error);
      console.log("No root doc found, initialising new tonk");
      tonk = await TonkCore.create();
      const newBytes = await tonk.toBytes(); 
      console.log(newBytes);
      fs.writeFileSync('output.txt', newBytes);
      await tonk.connectWebsocket('ws://localhost:8081');
      console.log('Connected to Tonk server');
    }
    if (await tonk.exists('/app')) {
      await tonk.deleteFile('/app');
      console.log("------- DELETED !!! -------");
    }
    if (await tonk.exists('/app/index.html')) { 
      console.log("already exists :)");
      console.log(await tonk.listDirectory('/app'));
      return;
    }
    else {
      // 5. Create directory structure
      await tonk.createDirectory('/app');
  
      console.log('Created directory structure');
      
      // 6. upload files using tonk file system :o
      const distPath = './eg-app-2';
      const basePath = '/app';
  
      await loadDistFiles(distPath, basePath); 
      
      console.log('✅ All files uploaded successfully!');
      console.log('🚀 Your React app should now be available');
    }

  } catch (error) {
    console.error('Error uploading files:', error);
  }
}

// Run the upload
await uploadFilesToTonk();
console.log("all uploaded :)")

console.log(tonk.listDirectory('/'));
console.log(await tonk.listDirectory('/app'));
