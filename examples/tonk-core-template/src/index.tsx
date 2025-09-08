import React from 'react';
import './index.css';
import { createRoot } from 'react-dom/client';
import { BrowserRouter } from 'react-router-dom';
import App from './App';
import { createTonk, TonkCore } from '@tonk/core';

const tonk1 = await createTonk();
const vfs1 = await tonk1.getVfs();

await vfs1.createFile('/hello.txt', 'Hello, World!');
const content1 = await vfs1.readFile('/hello.txt');
console.log('Initial content:', content1);

const data = await tonk1.toBytes();
const tonk2 = await TonkCore.fromBytes(data);
const vfs2 = await tonk2.getVfs();

const content2 = await vfs2.readFile('/hello.txt');
console.log('Retrieved content:', content2);

const container = document.getElementById('root');
if (!container) throw new Error('Failed to find the root element');
const root = createRoot(container);

const basename =
  import.meta.env.VITE_BASE_PATH !== '/'
    ? import.meta.env.VITE_BASE_PATH?.replace(/\/$/, '')
    : '';

root.render(
  <React.StrictMode>
    <BrowserRouter basename={basename}>
      <App />
    </BrowserRouter>
  </React.StrictMode>
);
