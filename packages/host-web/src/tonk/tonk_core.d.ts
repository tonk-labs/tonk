/* tslint:disable */
/* eslint-disable */
export function create_tonk_from_bundle(bundle: WasmBundle): Promise<any>;
export function create_tonk_from_bytes(data: Uint8Array): Promise<any>;
export function create_bundle_from_bytes(data: Uint8Array): WasmBundle;
export function create_tonk(): Promise<any>;
export function create_tonk_from_bundle_with_storage(bundle: WasmBundle, use_indexed_db: boolean): Promise<any>;
export function create_tonk_from_bytes_with_storage(data: Uint8Array, use_indexed_db: boolean): Promise<any>;
export function create_tonk_with_config(peer_id: string, use_indexed_db: boolean): Promise<any>;
export function create_tonk_with_storage(use_indexed_db: boolean): Promise<any>;
export function init(): void;
export function create_tonk_with_peer_id(peer_id: string): Promise<any>;
export function set_time_provider(callback: Function): void;
export class WasmBundle {
  private constructor();
  free(): void;
  static fromBytes(data: Uint8Array): WasmBundle;
  getPrefix(prefix: string): Promise<any>;
  getRootId(): Promise<any>;
  getManifest(): Promise<any>;
  get(key: string): Promise<any>;
  toBytes(): Promise<any>;
  listKeys(): Promise<any>;
}
export class WasmDocumentWatcher {
  private constructor();
  free(): void;
  documentId(): string;
  stop(): Promise<any>;
}
export class WasmTonkCore {
  free(): void;
  static fromBytes(data: Uint8Array): Promise<any>;
  createFile(path: string, content: any): Promise<any>;
  deleteFile(path: string): Promise<any>;
  static fromBundle(bundle: WasmBundle): Promise<any>;
  getPeerId(): Promise<any>;
  updateFile(path: string, content: any): Promise<any>;
  getMetadata(path: string): Promise<any>;
  static withPeerId(peer_id: string): Promise<any>;
  listDirectory(path: string): Promise<any>;
  watchDocument(path: string, callback: Function): Promise<any>;
  watchDirectory(path: string, callback: Function): Promise<any>;
  createDirectory(path: string): Promise<any>;
  connectWebsocket(url: string): Promise<any>;
  createFileWithBytes(path: string, content: any, bytes: Uint8Array): Promise<any>;
  updateFileWithBytes(path: string, content: any, bytes: Uint8Array): Promise<any>;
  constructor();
  exists(path: string): Promise<any>;
  toBytes(): Promise<any>;
  readFile(path: string): Promise<any>;
}

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
  readonly memory: WebAssembly.Memory;
  readonly __wbg_wasmbundle_free: (a: number, b: number) => void;
  readonly __wbg_wasmdocumentwatcher_free: (a: number, b: number) => void;
  readonly __wbg_wasmtonkcore_free: (a: number, b: number) => void;
  readonly create_bundle_from_bytes: (a: any) => [number, number, number];
  readonly create_tonk_from_bundle: (a: number) => any;
  readonly create_tonk_from_bundle_with_storage: (a: number, b: number) => any;
  readonly create_tonk_from_bytes_with_storage: (a: any, b: number) => any;
  readonly create_tonk_with_config: (a: number, b: number, c: number) => any;
  readonly create_tonk_with_peer_id: (a: number, b: number) => any;
  readonly create_tonk_with_storage: (a: number) => any;
  readonly wasmbundle_get: (a: number, b: number, c: number) => any;
  readonly wasmbundle_getManifest: (a: number) => any;
  readonly wasmbundle_getPrefix: (a: number, b: number, c: number) => any;
  readonly wasmbundle_getRootId: (a: number) => any;
  readonly wasmbundle_listKeys: (a: number) => any;
  readonly wasmbundle_toBytes: (a: number) => any;
  readonly wasmdocumentwatcher_documentId: (a: number) => [number, number];
  readonly wasmdocumentwatcher_stop: (a: number) => any;
  readonly wasmtonkcore_connectWebsocket: (a: number, b: number, c: number) => any;
  readonly wasmtonkcore_createDirectory: (a: number, b: number, c: number) => any;
  readonly wasmtonkcore_createFile: (a: number, b: number, c: number, d: any) => any;
  readonly wasmtonkcore_createFileWithBytes: (a: number, b: number, c: number, d: any, e: number, f: number) => any;
  readonly wasmtonkcore_deleteFile: (a: number, b: number, c: number) => any;
  readonly wasmtonkcore_exists: (a: number, b: number, c: number) => any;
  readonly wasmtonkcore_fromBundle: (a: number) => any;
  readonly wasmtonkcore_getMetadata: (a: number, b: number, c: number) => any;
  readonly wasmtonkcore_getPeerId: (a: number) => any;
  readonly wasmtonkcore_listDirectory: (a: number, b: number, c: number) => any;
  readonly wasmtonkcore_readFile: (a: number, b: number, c: number) => any;
  readonly wasmtonkcore_toBytes: (a: number) => any;
  readonly wasmtonkcore_updateFile: (a: number, b: number, c: number, d: any) => any;
  readonly wasmtonkcore_updateFileWithBytes: (a: number, b: number, c: number, d: any, e: number, f: number) => any;
  readonly wasmtonkcore_watchDirectory: (a: number, b: number, c: number, d: any) => any;
  readonly wasmtonkcore_watchDocument: (a: number, b: number, c: number, d: any) => any;
  readonly init: () => void;
  readonly create_tonk_from_bytes: (a: any) => any;
  readonly wasmtonkcore_fromBytes: (a: any) => any;
  readonly create_tonk: () => any;
  readonly wasmtonkcore_new: () => any;
  readonly wasmbundle_fromBytes: (a: any) => [number, number, number];
  readonly wasmtonkcore_withPeerId: (a: number, b: number) => any;
  readonly set_time_provider: (a: any) => void;
  readonly __wbindgen_malloc: (a: number, b: number) => number;
  readonly __wbindgen_realloc: (a: number, b: number, c: number, d: number) => number;
  readonly __wbindgen_exn_store: (a: number) => void;
  readonly __externref_table_alloc: () => number;
  readonly __wbindgen_export_4: WebAssembly.Table;
  readonly __wbindgen_free: (a: number, b: number, c: number) => void;
  readonly __wbindgen_export_6: WebAssembly.Table;
  readonly __externref_table_dealloc: (a: number) => void;
  readonly closure246_externref_shim: (a: number, b: number, c: any) => void;
  readonly closure250_externref_shim_multivalue_shim: (a: number, b: number, c: any) => [number, number];
  readonly closure754_externref_shim: (a: number, b: number, c: any) => void;
  readonly _dyn_core__ops__function__FnMut_____Output___R_as_wasm_bindgen__closure__WasmClosure___describe__invoke__h4939de6411d337f8: (a: number, b: number) => void;
  readonly closure771_externref_shim: (a: number, b: number, c: any) => void;
  readonly closure1070_externref_shim: (a: number, b: number, c: any, d: any) => void;
  readonly __wbindgen_start: () => void;
}

export type SyncInitInput = BufferSource | WebAssembly.Module;
/**
* Instantiates the given `module`, which can either be bytes or
* a precompiled `WebAssembly.Module`.
*
* @param {{ module: SyncInitInput }} module - Passing `SyncInitInput` directly is deprecated.
*
* @returns {InitOutput}
*/
export function initSync(module: { module: SyncInitInput } | SyncInitInput): InitOutput;

/**
* If `module_or_path` is {RequestInfo} or {URL}, makes a request and
* for everything else, calls `WebAssembly.instantiate` directly.
*
* @param {{ module_or_path: InitInput | Promise<InitInput> }} module_or_path - Passing `InitInput` directly is deprecated.
*
* @returns {Promise<InitOutput>}
*/
export default function __wbg_init (module_or_path?: { module_or_path: InitInput | Promise<InitInput> } | InitInput | Promise<InitInput>): Promise<InitOutput>;
