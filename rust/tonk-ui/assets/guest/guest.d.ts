/* tslint:disable */
/* eslint-disable */
/**
 * The `ReadableStreamType` enum.
 *
 * *This API requires the following crate features to be activated: `ReadableStreamType`*
 */

type ReadableStreamType = "bytes";

/**
 * The predicate of a semantic triple
 */
type Attribute = string;

/**
 * The subject of a semantic triple; it must be exactly 32-bytes long.
 * A valid, unique `Entity` can be created using `generateEntity`.
 */
type Entity = Uint8Array;

/**
 * The object of a semantic triple. It's internal representation will
 * vary based on the value of the `type` property. For more details,
 * see the documentation on `ValueDataType`.
 */
interface Value {
    type: ValueDataType,
    value: null|Uint8Array|string|boolean|number
}

/**
 * A causal reference to an earlier version of an `Artifact`
 */
type Cause = Uint8Array;

/**
 * An `Artifact` embodies a datum - a semantic triple - that may be stored in or
 * retrieved from `Artifacts`.
 */
interface Artifact {
    the: Attribute,
    of: Entity,
    is: Value,
    cause?: Cause
}

interface ArtifactApi {
    update(value: Value): (Artifact & ArtifactApi)|void;
}

/**
 * The instruction variants that are accepted by `Artifacts.commit`.
 */
interface Instruction {
    type: InstructionType,
    artifact: Artifact
}

/**
 * The shape of the "iterable" that is expected by `Artifacts.commit`
 */
type InstructionIterable = Iterable<Instruction>;

/**
 * A basic filter that can be used to query `Artifacts`
 */
interface ArtifactSelector {
    the?: Attribute,
    of?: Entity,
    is?: Value
}

/**
 * The shape of the "async iterable" that is returned by `Artifacts.select`
 */
type ArtifactIterable = AsyncIterable<Artifact & ArtifactApi>;



/**
 * An async iterator that lazily yields `Artifact`s
 */
export class ArtifactIterator {
    private constructor();
    free(): void;
    [Symbol.dispose](): void;
    /**
     * Get the next `Artifact` yielded by this iterator
     */
    next(): Promise<IteratorResult<Artifact>>;
}

/**
 * A triple store that can be used to store and retrieve semantic triples
 * in the form of `Artifact`s.
 */
export class Artifacts {
    private constructor();
    free(): void;
    [Symbol.dispose](): void;
    /**
     * Initialize a new, empty [`Artifacts`] with a randomly generated
     * identifier
     */
    static anonymous(): Promise<Artifacts>;
    /**
     * Persist a set of data in the triple store. The returned prommise
     * resolves when all data has been persisted and the revision has been
     * updated. Any data that does not match the expected shape of an
     * `Artifact` is quietly ignored (this is probably bad, but it is
     * expedient). If there is an error during the commit, the change is
     * abandoned and the revision remains the same as it was at the start of
     * the transaction.
     */
    commit(iterable: InstructionIterable): Promise<Uint8Array>;
    /**
     * The name used to uniquely identify the data of this [`Artifacts`]
     * instance
     */
    identifier(): Promise<string>;
    /**
     * Construct a new `Artifacts`, backed by a database. If the same name is
     * used for multiple instances (or across sessions), the same database will
     * be used.
     */
    static open(identifier: string): Promise<Artifacts>;
    /**
     * Reset the root of the database to `revision` if provided, or else reset
     * to the stored root if available, or else to an empty database.
     */
    reset(revision?: Uint8Array | null): Promise<void>;
    /**
     * Get the current revision of the triple store. This value will change on
     * every successful call to `Artifacts.commit`. The returned value is
     * suitable for use with `Artifacts.restore`, for example when re-opening
     * the triple store on future sessions.
     */
    revision(): Promise<Uint8Array>;
    /**
     * Query for `Artifact`s that match the given selector. Matching results
     * are provided via an async iterator.
     */
    select(selector: ArtifactSelector): ArtifactIterable;
}

/**
 * Used to specify if an `Instruction` is an assertion or a retraction
 */
export enum InstructionType {
    /**
     * The `Instruction` is an assertion
     */
    Assert = 0,
    /**
     * The `Instruction` is a retraction
     */
    Retract = 1,
}

export class IntoUnderlyingByteSource {
    private constructor();
    free(): void;
    [Symbol.dispose](): void;
    cancel(): void;
    pull(controller: ReadableByteStreamController): Promise<any>;
    start(controller: ReadableByteStreamController): void;
    readonly autoAllocateChunkSize: number;
    readonly type: ReadableStreamType;
}

export class IntoUnderlyingSink {
    private constructor();
    free(): void;
    [Symbol.dispose](): void;
    abort(reason: any): Promise<any>;
    close(): Promise<any>;
    write(chunk: any): Promise<any>;
}

export class IntoUnderlyingSource {
    private constructor();
    free(): void;
    [Symbol.dispose](): void;
    cancel(): void;
    pull(controller: ReadableStreamDefaultController): Promise<any>;
}

/**
 * [`ValueDataType`] embodies all types that are able to be represented
 * as a [`Value`].
 */
export enum ValueDataType {
    /**
     * A byte buffer
     */
    Bytes = 0,
    /**
     * An [`Entity`]
     */
    Entity = 1,
    /**
     * A boolean
     */
    Boolean = 2,
    /**
     * A UTF-8 string
     */
    String = 3,
    /**
     * A 128-bit unsigned integer
     */
    UnsignedInt = 4,
    /**
     * A 128-bit signed integer
     */
    SignedInt = 5,
    /**
     * A floating point number
     */
    Float = 6,
    /**
     * TBD structured data (flatbuffers?)
     */
    Record = 7,
    /**
     * A symbol type, used to distinguish attributes from other strings
     */
    Symbol = 8,
}

/**
 * Decode base58-encoded bytes from a string
 */
export function decode(encoded: string): Uint8Array;

/**
 * Convert the input bytes to a string using base58 encoding
 */
export function encode(bytes: Uint8Array): string;

/**
 * Generate a unique, valid `Entity`
 */
export function generateEntity(): Uint8Array;

/**
 * Generate the BLAKE3 hash of some input bytes
 */
export function makeReference(bytes: Uint8Array): Uint8Array;

/**
 * Register the guest's custom elements. Call once, after wasm init, from
 * the guest bootstrap. Idempotent.
 */
export function start(): void;

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
    readonly memory: WebAssembly.Memory;
    readonly __wbg_artifactiterator_free: (a: number, b: number) => void;
    readonly __wbg_artifacts_free: (a: number, b: number) => void;
    readonly artifactiterator_next: (a: number) => any;
    readonly artifacts_anonymous: () => any;
    readonly artifacts_commit: (a: number, b: any) => any;
    readonly artifacts_identifier: (a: number) => any;
    readonly artifacts_open: (a: number, b: number) => any;
    readonly artifacts_reset: (a: number, b: number, c: number) => any;
    readonly artifacts_revision: (a: number) => any;
    readonly artifacts_select: (a: number, b: any) => [number, number, number];
    readonly decode: (a: number, b: number) => [number, number, number, number];
    readonly encode: (a: number, b: number) => [number, number];
    readonly generateEntity: () => [number, number, number, number];
    readonly makeReference: (a: number, b: number) => [number, number];
    readonly start: () => void;
    readonly main: (a: number, b: number) => number;
    readonly __wbg_intounderlyingbytesource_free: (a: number, b: number) => void;
    readonly __wbg_intounderlyingsink_free: (a: number, b: number) => void;
    readonly __wbg_intounderlyingsource_free: (a: number, b: number) => void;
    readonly intounderlyingbytesource_autoAllocateChunkSize: (a: number) => number;
    readonly intounderlyingbytesource_cancel: (a: number) => void;
    readonly intounderlyingbytesource_pull: (a: number, b: any) => any;
    readonly intounderlyingbytesource_start: (a: number, b: any) => void;
    readonly intounderlyingbytesource_type: (a: number) => number;
    readonly intounderlyingsink_abort: (a: number, b: any) => any;
    readonly intounderlyingsink_close: (a: number) => any;
    readonly intounderlyingsink_write: (a: number, b: any) => any;
    readonly intounderlyingsource_cancel: (a: number) => void;
    readonly intounderlyingsource_pull: (a: number, b: any) => any;
    readonly wasm_bindgen__closure__destroy__h49d5fa68e10aed29: (a: number, b: number) => void;
    readonly wasm_bindgen__closure__destroy__hd813ad2ee5c5c4a2: (a: number, b: number) => void;
    readonly wasm_bindgen__closure__destroy__h079abaa5c345a56d: (a: number, b: number) => void;
    readonly wasm_bindgen__closure__destroy__h0dbe9a2cc03dd049: (a: number, b: number) => void;
    readonly wasm_bindgen__closure__destroy__hb663ece502a44c6b: (a: number, b: number) => void;
    readonly wasm_bindgen__closure__destroy__h82f2e521aaa87d98: (a: number, b: number) => void;
    readonly wasm_bindgen__closure__destroy__h088a256bb860b3fe: (a: number, b: number) => void;
    readonly wasm_bindgen__closure__destroy__h693a7f98abc55f77: (a: number, b: number) => void;
    readonly wasm_bindgen__closure__destroy__h2f05097e2b37f73e: (a: number, b: number) => void;
    readonly wasm_bindgen__closure__destroy__h71e65fef89e6fff2: (a: number, b: number) => void;
    readonly wasm_bindgen__closure__destroy__h0716a3841d988ca7: (a: number, b: number) => void;
    readonly wasm_bindgen__convert__closures_____invoke__h73173eb645636614: (a: number, b: number, c: any, d: number, e: number, f: number, g: number, h: number, i: number) => void;
    readonly wasm_bindgen__convert__closures_____invoke__hd8c48394ab3e3e2a: (a: number, b: number, c: any, d: number, e: number, f: number, g: number, h: number, i: number) => void;
    readonly wasm_bindgen__convert__closures_____invoke__h284e952984467d5b: (a: number, b: number, c: any, d: number, e: number, f: number, g: number, h: number, i: number) => void;
    readonly wasm_bindgen__convert__closures_____invoke__hd89e8bfaa5c05553: (a: number, b: number, c: any, d: number, e: number, f: number, g: number, h: number, i: number) => void;
    readonly wasm_bindgen__convert__closures_____invoke__h81e64a2557ee8bac: (a: number, b: number, c: any, d: any) => void;
    readonly wasm_bindgen__convert__closures_____invoke__h5f727dde042f55d6: (a: number, b: number, c: any, d: any) => void;
    readonly wasm_bindgen__convert__closures_____invoke__h1b0dc3fcdf71fa0a: (a: number, b: number, c: any, d: any) => void;
    readonly wasm_bindgen__convert__closures_____invoke__h76571fedcc76ac2c: (a: number, b: number, c: any) => void;
    readonly wasm_bindgen__convert__closures_____invoke__he0f225171d5f12cb: (a: number, b: number, c: any) => void;
    readonly wasm_bindgen__convert__closures_____invoke__h388031cba1fd214c: (a: number, b: number, c: any) => void;
    readonly wasm_bindgen__convert__closures_____invoke__h415b30baa72d2653: (a: number, b: number, c: any) => void;
    readonly wasm_bindgen__convert__closures_____invoke__h4fb10bd9bc9dc57e: (a: number, b: number, c: any) => void;
    readonly wasm_bindgen__convert__closures_____invoke__h03d009306f090cf6: (a: number, b: number, c: any) => void;
    readonly wasm_bindgen__convert__closures_____invoke__h389482a9fb106f9a: (a: number, b: number, c: any) => void;
    readonly wasm_bindgen__convert__closures_____invoke__h2f91fd9e01bff769: (a: number, b: number) => number;
    readonly wasm_bindgen__convert__closures_____invoke__hce1fb97672fd939d: (a: number, b: number) => void;
    readonly wasm_bindgen__convert__closures_____invoke__h4c5e7d9046c92e17: (a: number, b: number) => void;
    readonly __wbindgen_malloc: (a: number, b: number) => number;
    readonly __wbindgen_realloc: (a: number, b: number, c: number, d: number) => number;
    readonly __wbindgen_exn_store: (a: number) => void;
    readonly __externref_table_alloc: () => number;
    readonly __wbindgen_externrefs: WebAssembly.Table;
    readonly __wbindgen_free: (a: number, b: number, c: number) => void;
    readonly __externref_table_dealloc: (a: number) => void;
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
