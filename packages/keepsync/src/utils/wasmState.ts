import { initializeWasm } from '@automerge/automerge-repo/slim';

let isSlim = false;

export async function waitForWasm(): Promise<void> {
  if (isSlim) return;
  await initializeWasm(
    'https://esm.sh/@automerge/automerge@3.1.1/dist/automerge.wasm'
  );
}

export function setIsSlim() {
  isSlim = true;
}
