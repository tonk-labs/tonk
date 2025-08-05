import { initializeWasm } from '@automerge/automerge-repo/slim';
import wasmUrl from '@automerge/automerge/automerge.wasm?url';

let isSlim = false;

export async function waitForWasm(): Promise<void> {
  if (isSlim) return;
  await initializeWasm(wasmUrl);
}

export function setIsSlim() {
  isSlim = true;
}
