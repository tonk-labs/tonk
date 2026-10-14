(() => {
  var __defProp = Object.defineProperty;
  var __export = (target, all) => {
    for (var name in all)
      __defProp(target, name, { get: all[name], enumerable: true });
  };

  // snippets/tonk-identity-007d8aaa5038d82a/src/custody-channel.mjs
  function startCustodyProgress(port, intervalMs = 1e4) {
    const progress = () => port.postMessage({ pending: true });
    progress();
    const timer = setInterval(progress, intervalMs);
    return () => clearInterval(timer);
  }

  // snippets/tonk-worker-c0482e79796fa9a7/inline0.js
  async function space_storage_exists(name) {
    const databases = await indexedDB.databases();
    return databases.some((database) => database.name === name);
  }

  // snippets/tonk-worker-c0482e79796fa9a7/inline1.js
  function delete_space_storage(name) {
    const database = new Promise((resolve) => {
      const request = indexedDB.deleteDatabase(name);
      request.onsuccess = request.onerror = () => resolve();
      request.onblocked = () => setTimeout(resolve, 1500);
    });
    const blobs = navigator.storage.getDirectory().then((root) => root.getDirectoryHandle("current")).then((dir) => dir.removeEntry(name, { recursive: true })).catch(() => {
    });
    return Promise.all([database, blobs]);
  }

  // snippets/tonk-worker-c0482e79796fa9a7/inline2.js
  var inline2_exports = {};
  __export(inline2_exports, {
    patch_idb_dead_shims: () => patch_idb_dead_shims,
    patch_idb_versionchange: () => patch_idb_versionchange
  });
  function patch_idb_versionchange() {
    const desc = Object.getOwnPropertyDescriptor(
      IDBDatabase.prototype,
      "onversionchange"
    );
    if (!desc || !desc.set) return;
    const origSet = desc.set;
    Object.defineProperty(IDBDatabase.prototype, "onversionchange", {
      set(handler) {
        origSet.call(this, function() {
          this.close();
        });
      },
      get: desc.get,
      configurable: true
    });
  }
  function patch_idb_dead_shims() {
    const guard = (proto, name) => {
      const desc = Object.getOwnPropertyDescriptor(proto, name);
      if (!desc || !desc.set) return;
      const origSet = desc.set;
      Object.defineProperty(proto, name, {
        set(handler) {
          if (typeof handler !== "function") {
            origSet.call(this, handler);
            return;
          }
          origSet.call(this, function(...args) {
            try {
              return handler.apply(this, args);
            } catch (e) {
              if (String(e && e.message || e).includes("closure invoked")) {
                console.debug(
                  `idb ${name}: event for a torn-down wasm instance ignored`
                );
                return;
              }
              throw e;
            }
          });
        },
        get: desc.get,
        configurable: true
      });
    };
    for (const name of ["oncomplete", "onerror", "onabort"]) {
      guard(IDBTransaction.prototype, name);
    }
    for (const name of ["onsuccess", "onerror"]) {
      guard(IDBRequest.prototype, name);
    }
    guard(IDBOpenDBRequest.prototype, "onupgradeneeded");
  }

  // worker.js
  var import_meta = {};
  var IntoUnderlyingByteSource = class {
    __destroy_into_raw() {
      const ptr = this.__wbg_ptr;
      this.__wbg_ptr = 0;
      IntoUnderlyingByteSourceFinalization.unregister(this);
      return ptr;
    }
    free() {
      const ptr = this.__destroy_into_raw();
      wasm.__wbg_intounderlyingbytesource_free(ptr, 0);
    }
    /**
     * @returns {number}
     */
    get autoAllocateChunkSize() {
      const ret = wasm.intounderlyingbytesource_autoAllocateChunkSize(this.__wbg_ptr);
      return ret >>> 0;
    }
    cancel() {
      const ptr = this.__destroy_into_raw();
      wasm.intounderlyingbytesource_cancel(ptr);
    }
    /**
     * @param {ReadableByteStreamController} controller
     * @returns {Promise<any>}
     */
    pull(controller) {
      const ret = wasm.intounderlyingbytesource_pull(this.__wbg_ptr, controller);
      return ret;
    }
    /**
     * @param {ReadableByteStreamController} controller
     */
    start(controller) {
      wasm.intounderlyingbytesource_start(this.__wbg_ptr, controller);
    }
    /**
     * @returns {ReadableStreamType}
     */
    get type() {
      const ret = wasm.intounderlyingbytesource_type(this.__wbg_ptr);
      return __wbindgen_enum_ReadableStreamType[ret];
    }
  };
  if (Symbol.dispose) IntoUnderlyingByteSource.prototype[Symbol.dispose] = IntoUnderlyingByteSource.prototype.free;
  var IntoUnderlyingSink = class {
    __destroy_into_raw() {
      const ptr = this.__wbg_ptr;
      this.__wbg_ptr = 0;
      IntoUnderlyingSinkFinalization.unregister(this);
      return ptr;
    }
    free() {
      const ptr = this.__destroy_into_raw();
      wasm.__wbg_intounderlyingsink_free(ptr, 0);
    }
    /**
     * @param {any} reason
     * @returns {Promise<any>}
     */
    abort(reason) {
      const ptr = this.__destroy_into_raw();
      const ret = wasm.intounderlyingsink_abort(ptr, reason);
      return ret;
    }
    /**
     * @returns {Promise<any>}
     */
    close() {
      const ptr = this.__destroy_into_raw();
      const ret = wasm.intounderlyingsink_close(ptr);
      return ret;
    }
    /**
     * @param {any} chunk
     * @returns {Promise<any>}
     */
    write(chunk) {
      const ret = wasm.intounderlyingsink_write(this.__wbg_ptr, chunk);
      return ret;
    }
  };
  if (Symbol.dispose) IntoUnderlyingSink.prototype[Symbol.dispose] = IntoUnderlyingSink.prototype.free;
  var IntoUnderlyingSource = class _IntoUnderlyingSource {
    static __wrap(ptr) {
      const obj = Object.create(_IntoUnderlyingSource.prototype);
      obj.__wbg_ptr = ptr;
      IntoUnderlyingSourceFinalization.register(obj, obj.__wbg_ptr, obj);
      return obj;
    }
    __destroy_into_raw() {
      const ptr = this.__wbg_ptr;
      this.__wbg_ptr = 0;
      IntoUnderlyingSourceFinalization.unregister(this);
      return ptr;
    }
    free() {
      const ptr = this.__destroy_into_raw();
      wasm.__wbg_intounderlyingsource_free(ptr, 0);
    }
    cancel() {
      const ptr = this.__destroy_into_raw();
      wasm.intounderlyingsource_cancel(ptr);
    }
    /**
     * @param {ReadableStreamDefaultController} controller
     * @returns {Promise<any>}
     */
    pull(controller) {
      const ret = wasm.intounderlyingsource_pull(this.__wbg_ptr, controller);
      return ret;
    }
  };
  if (Symbol.dispose) IntoUnderlyingSource.prototype[Symbol.dispose] = IntoUnderlyingSource.prototype.free;
  var TonkServiceWorker = class _TonkServiceWorker {
    static __wrap(ptr) {
      const obj = Object.create(_TonkServiceWorker.prototype);
      obj.__wbg_ptr = ptr;
      TonkServiceWorkerFinalization.register(obj, obj.__wbg_ptr, obj);
      return obj;
    }
    __destroy_into_raw() {
      const ptr = this.__wbg_ptr;
      this.__wbg_ptr = 0;
      TonkServiceWorkerFinalization.unregister(this);
      return ptr;
    }
    free() {
      const ptr = this.__destroy_into_raw();
      wasm.__wbg_tonkserviceworker_free(ptr, 0);
    }
    /**
     * Take up a delegation for `space` issued by the person's profile: save
     * its chain and mount the space as a replica syncing where the chain
     * was signed to say. Resolves to the `{ remote, account, name }` taken up.
     * @param {string} space
     * @param {Uint8Array} chain
     * @returns {Promise<any>}
     */
    adoptSpace(space, chain) {
      const ptr0 = passStringToWasm0(space, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
      const len0 = WASM_VECTOR_LEN;
      const ret = wasm.tonkserviceworker_adoptSpace(this.__wbg_ptr, ptr0, len0, chain);
      return ret;
    }
    /**
     * Create the content of `space`, which this worker holds, from the seed
     * the person's profile handed over (JSON, as [`pendingSeed`] gives it).
     *
     * [`pendingSeed`]: Self::pending_seed
     * @param {string} space
     * @param {string} seed
     * @returns {Promise<any>}
     */
    createContent(space, seed) {
      const ptr0 = passStringToWasm0(space, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
      const len0 = WASM_VECTOR_LEN;
      const ptr1 = passStringToWasm0(seed, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
      const len1 = WASM_VECTOR_LEN;
      const ret = wasm.tonkserviceworker_createContent(this.__wbg_ptr, ptr0, len0, ptr1, len1);
      return ret;
    }
    /**
     * Create `space`'s content, in this worker, as a copy of another
     * space's: `seed` is what the person's profile handed over (JSON, as
     * [`pendingSeed`] gives it), `content` and `revision` the other space's
     * snapshot from its own worker.
     *
     * [`pendingSeed`]: Self::pending_seed
     * @param {string} space
     * @param {string} seed
     * @param {Uint8Array} content
     * @param {Uint8Array} revision
     * @returns {Promise<any>}
     */
    createCopy(space, seed, content, revision) {
      const ptr0 = passStringToWasm0(space, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
      const len0 = WASM_VECTOR_LEN;
      const ptr1 = passStringToWasm0(seed, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
      const len1 = WASM_VECTOR_LEN;
      const ret = wasm.tonkserviceworker_createCopy(this.__wbg_ptr, ptr0, len0, ptr1, len1, content, revision);
      return ret;
    }
    /**
     * Issue the space worker whose profile is `audience` a delegation to use
     * `space`. Resolves to `{ chain, expires }`: the encoded chain, and when
     * it lapses (unix seconds). Where the space syncs and which account it
     * is for are signed into the chain.
     * @param {string} space
     * @param {string} audience
     * @returns {Promise<any>}
     */
    delegateSpace(space, audience) {
      const ptr0 = passStringToWasm0(space, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
      const len0 = WASM_VECTOR_LEN;
      const ptr1 = passStringToWasm0(audience, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
      const len1 = WASM_VECTOR_LEN;
      const ret = wasm.tonkserviceworker_delegateSpace(this.__wbg_ptr, ptr0, len0, ptr1, len1);
      return ret;
    }
    /**
     * Mint an invite to `space`, as though its share control had been
     * clicked at `time`. For a space whose own worker was asked for one: a
     * delegation that worker issued would lapse with its own.
     * @param {string} space
     * @param {number} time
     * @returns {Promise<any>}
     */
    inviteSpace(space, time) {
      const ptr0 = passStringToWasm0(space, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
      const len0 = WASM_VECTOR_LEN;
      const ret = wasm.tonkserviceworker_inviteSpace(this.__wbg_ptr, ptr0, len0, time);
      return ret;
    }
    /**
     * Put `name` on the roster entry of the account this worker acts for
     * in `space`, which it holds. Resolves to whether the roster changed.
     * @param {string} space
     * @param {string} name
     * @returns {Promise<any>}
     */
    nameMember(space, name) {
      const ptr0 = passStringToWasm0(space, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
      const len0 = WASM_VECTOR_LEN;
      const ptr1 = passStringToWasm0(name, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
      const len1 = WASM_VECTOR_LEN;
      const ret = wasm.tonkserviceworker_nameMember(this.__wbg_ptr, ptr0, len0, ptr1, len1);
      return ret;
    }
    /**
     * Creates a new service worker instance.
     *
     * Initializes the user profile and operator — no repositories
     * are opened or created here. Repositories are created
     * on-demand via `PUT /api/repository/{name}`; subsequent
     * requests against routes that expect the repository to exist
     * will 404 until that happens.
     *
     * # Errors
     *
     * Returns a `JsError` if the service worker cannot be initialized.
     * @returns {Promise<TonkServiceWorker>}
     */
    static new() {
      const ret = wasm.tonkserviceworker_new();
      return ret;
    }
    /**
     * Called from the JS shim's `self.onactivate` to resume this generation's
     * sync scheduler. Client adoption is page-directed: compatible pages ask
     * the activated worker to claim them, while older pages retain their
     * current controller and may still need that controller's shell and Wasm
     * caches. Activation therefore never purges another generation.
     * @returns {Promise<any>}
     */
    onactivate() {
      const ret = wasm.tonkserviceworker_onactivate(this.__wbg_ptr);
      return ret;
    }
    /**
     * Connectivity changed. Re-read `navigator.onLine` (reliable in the SW
     * scope) and reconcile: offline stamps `sync:offline` on every open repo
     * so the chips/discs reflect the disconnect; online runs a drain and
     * restarts the self-scheduled sync loop the offline transition stopped.
     *
     * Fired both by the SW's own `offline`/`online` events and by a
     * `{type:"connectivity"}` nudge from the active page (whose events fire
     * even when the SW's don't). Either way the decision is made from
     * `navigator.onLine`, not from who fired — a flapping transition settles
     * on the current reading.
     * @returns {Promise<any>}
     */
    onconnectivity() {
      const ret = wasm.tonkserviceworker_onconnectivity(this.__wbg_ptr);
      return ret;
    }
    /**
     * Handles incoming fetch events from the browser.
     *
     * Called from the JS shim for data-plane/live requests and for every
     * request whose initiating client may be a registered guest. Exact
     * top-level stamped documents/assets stay in the JS fast path; the Rust
     * side decides whether each delegated request belongs in the axum router
     * or must pass through to the network.
     *
     * Decision rule (see [`route_for`]):
     * - If the request's path starts with `/api/`, route through
     *   the axum router unchanged.
     * - If the initiating client is a registered guest iframe,
     *   rewrite the path to live under
     *   `/api/repository/{repo}/branch/{branch}` and route
     *   through the axum router. The iframe sees its branch as
     *   its virtual root.
     * - Otherwise, pass through to the network via `sw_fetch`.
     *
     * Navigation requests that pass through and 404 are retried
     * against `/index.html` so the client-side SPA router can
     * match unknown paths.
     * @param {FetchEvent} event
     * @returns {Promise<any>}
     */
    onfetch(event) {
      const ret = wasm.tonkserviceworker_onfetch(this.__wbg_ptr, event);
      return ret;
    }
    /**
     * Handles `message` events posted to the worker. Two envelopes
     * arrive this way: `custody`, carrying the PRF outputs of a passkey
     * ceremony, and `ceremony-refused`, the top-level page saying it
     * could not run a ceremony the account panel asked for. Queries,
     * subscriptions, transactions and evaluations never do: pages reach
     * those over the branch's HTTP routes.
     *
     * Wired into the SW global by the same JS bootstrap that sets
     * `self.onfetch`.
     * @param {ExtendableMessageEvent} event
     * @returns {Promise<any>}
     */
    onmessage(event) {
      const ret = wasm.tonkserviceworker_onmessage(this.__wbg_ptr, event);
      return ret;
    }
    /**
     * Hook the SW's `updatefound` event from JavaScript.
     *
     * When the registration sees a newer worker entering the
     * `installing` state, this active worker is on its way out.
     * We use the moment to close every long-lived stream we're
     * serving — `/api/lsp/events` SSE plus every `/query` SSE
     * subscription — so the in-flight fetch events settle and the outgoing
     * worker can terminate after the successor takes over.
     *
     * Without this the SW spec keeps the outgoing worker alive while any of
     * its fetches are open, allowing old-client reconnects to keep reaching
     * the retired generation after the successor activates.
     * @returns {Promise<any>}
     */
    onupdatefound() {
      const ret = wasm.tonkserviceworker_onupdatefound(this.__wbg_ptr);
      return ret;
    }
    /**
     * A page became visible: restore the active cadence and reconcile
     * immediately, instead of waiting out the hidden interval.
     * @returns {Promise<any>}
     */
    onvisibility() {
      const ret = wasm.tonkserviceworker_onvisibility(this.__wbg_ptr);
      return ret;
    }
    /**
     * What `space`'s own worker is to create its content from, as JSON, or
     * `null` when there is nothing left to hand over.
     * @param {string} space
     * @returns {Promise<any>}
     */
    pendingSeed(space) {
      const ptr0 = passStringToWasm0(space, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
      const len0 = WASM_VECTOR_LEN;
      const ret = wasm.tonkserviceworker_pendingSeed(this.__wbg_ptr, ptr0, len0);
      return ret;
    }
    /**
     * The DID of the profile this worker signs as. A space's own worker asks
     * for its delegation with it.
     * @returns {Promise<any>}
     */
    profileDid() {
      const ret = wasm.tonkserviceworker_profileDid(this.__wbg_ptr);
      return ret;
    }
    /**
     * Keep none of `space`'s content here: its own origin's worker has said
     * it holds it. `push_first` when that worker filled from the remote and
     * not from this one. Resolves to whether anything was released.
     * @param {string} space
     * @param {boolean} push_first
     * @returns {Promise<any>}
     */
    releaseSpace(space, push_first) {
      const ptr0 = passStringToWasm0(space, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
      const len0 = WASM_VECTOR_LEN;
      const ret = wasm.tonkserviceworker_releaseSpace(this.__wbg_ptr, ptr0, len0, push_first);
      return ret;
    }
    /**
     * Make again the site stamps a previous instance saved, for the clients
     * in `live` (the ids `clients.matchAll()` lists now). Saved state this
     * worker cannot read is ignored: its pages claim their sites again.
     * @param {Uint8Array} bytes
     * @param {string[]} live
     * @returns {Promise<any>}
     */
    restoreSession(bytes, live) {
      const ptr0 = passArrayJsValueToWasm0(live, wasm.__wbindgen_malloc);
      const len0 = WASM_VECTOR_LEN;
      const ret = wasm.tonkserviceworker_restoreSession(this.__wbg_ptr, bytes, ptr0, len0);
      return ret;
    }
    /**
     * Put back what a restart of this worker lost about `space`, which it
     * holds: who its session acts for. Called each time a space's own
     * worker starts.
     * @param {string} space
     * @returns {Promise<any>}
     */
    resumeSpace(space) {
      const ptr0 = passStringToWasm0(space, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
      const len0 = WASM_VECTOR_LEN;
      const ret = wasm.tonkserviceworker_resumeSpace(this.__wbg_ptr, ptr0, len0);
      return ret;
    }
    /**
     * Sign and publish the revocation of a grant on `space` that its own
     * worker asked for: `request` is what that worker sent, as JSON.
     * Resolves to the access service's receipt, as JSON.
     * @param {string} space
     * @param {string} request
     * @returns {Promise<any>}
     */
    revokeGrant(space, request) {
      const ptr0 = passStringToWasm0(space, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
      const len0 = WASM_VECTOR_LEN;
      const ptr1 = passStringToWasm0(request, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
      const len1 = WASM_VECTOR_LEN;
      const ret = wasm.tonkserviceworker_revokeGrant(this.__wbg_ptr, ptr0, len0, ptr1, len1);
      return ret;
    }
    /**
     * The site stamps this worker holds, as bytes for the next instance to
     * [`restore_session`](Self::restore_session) from. Equal state yields
     * equal bytes, so the caller can skip saving what has not changed.
     * @returns {Promise<any>}
     */
    savedSession() {
      const ret = wasm.tonkserviceworker_savedSession(this.__wbg_ptr);
      return ret;
    }
    /**
     * Seed this worker's freshly mounted replica of `space` from the host's
     * snapshot. A replica that already has content is left alone.
     * @param {string} space
     * @param {Uint8Array} content
     * @param {Uint8Array} revision
     * @returns {Promise<any>}
     */
    seedSpace(space, content, revision) {
      const ptr0 = passStringToWasm0(space, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
      const len0 = WASM_VECTOR_LEN;
      const ret = wasm.tonkserviceworker_seedSpace(this.__wbg_ptr, ptr0, len0, content, revision);
      return ret;
    }
    /**
     * `space`'s own worker has created its content from the seed.
     * @param {string} space
     * @returns {Promise<any>}
     */
    settleSeed(space) {
      const ptr0 = passStringToWasm0(space, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
      const len0 = WASM_VECTOR_LEN;
      const ret = wasm.tonkserviceworker_settleSeed(this.__wbg_ptr, ptr0, len0);
      return ret;
    }
    /**
     * Snapshot `space`'s `main` for its own worker to seed from. Resolves to
     * `{ content, revision }` (a CARv1 and the revision as JSON), or `null`
     * when `main` has nothing on it yet.
     * @param {string} space
     * @returns {Promise<any>}
     */
    snapshotSpace(space) {
      const ptr0 = passStringToWasm0(space, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
      const len0 = WASM_VECTOR_LEN;
      const ret = wasm.tonkserviceworker_snapshotSpace(this.__wbg_ptr, ptr0, len0);
      return ret;
    }
    /**
     * The person's account's authority over `space`, as this profile has
     * it on record, for the space's own worker to compose onto. Resolves
     * to the chain, base58.
     * @param {string} space
     * @returns {Promise<any>}
     */
    spaceAuthority(space) {
      const ptr0 = passStringToWasm0(space, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
      const len0 = WASM_VECTOR_LEN;
      const ret = wasm.tonkserviceworker_spaceAuthority(this.__wbg_ptr, ptr0, len0);
      return ret;
    }
    /**
     * Where `space` syncs and which account this profile acts for, as its
     * own worker last has to have taken them up. Resolves to
     * `{ remote, account, name }`, the remote `null` for a space that only exists
     * here.
     * @param {string} space
     * @returns {Promise<any>}
     */
    spaceTerms(space) {
      const ptr0 = passStringToWasm0(space, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
      const len0 = WASM_VECTOR_LEN;
      const ret = wasm.tonkserviceworker_spaceTerms(this.__wbg_ptr, ptr0, len0);
      return ret;
    }
    /**
     * Runs a durable background sync, invoked from the Background Sync API
     * `onsync` event — the tab-closed backstop for the in-fetch drain.
     *
     * The SW owns the sync work-queue (repos with un-pushed commits) and knows
     * every open repo, so the event needs no per-repo identity: the tag is a
     * single bare `"sync"`, ignored here. This funnels to the same
     * [`drain_sync`](crate::router::drain_sync) as `POST /api/sync` and the
     * per-fetch `event.waitUntil` drain — push the dirty set, pull every open
     * repo.
     *
     * # Returns
     *
     * A JavaScript `Promise` that resolves to `undefined` once the drain
     * completes.
     * @param {string} tag
     * @returns {Promise<any>}
     */
    sync(tag) {
      const ptr0 = passStringToWasm0(tag, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
      const len0 = WASM_VECTOR_LEN;
      const ret = wasm.tonkserviceworker_sync(this.__wbg_ptr, ptr0, len0);
      return ret;
    }
  };
  if (Symbol.dispose) TonkServiceWorker.prototype[Symbol.dispose] = TonkServiceWorker.prototype.free;
  function activate(build_id, asset_paths, site) {
    const ptr0 = passStringToWasm0(build_id, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len0 = WASM_VECTOR_LEN;
    const ret = wasm.activate(ptr0, len0, asset_paths, site);
    return ret;
  }
  function __wbg_get_imports() {
    const import0 = {
      __proto__: null,
      __wbg_Error_92b29b0548f8b746: function(arg0, arg1) {
        const ret = Error(getStringFromWasm0(arg0, arg1));
        return ret;
      },
      __wbg_String_8564e559799eccda: function(arg0, arg1) {
        const ret = String(arg1);
        const ptr1 = passStringToWasm0(ret, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len1 = WASM_VECTOR_LEN;
        getDataViewMemory0().setInt32(arg0 + 4 * 1, len1, true);
        getDataViewMemory0().setInt32(arg0 + 4 * 0, ptr1, true);
      },
      __wbg___wbindgen_bigint_get_as_i64_d968e41184ae354f: function(arg0, arg1) {
        const v = arg1;
        const ret = typeof v === "bigint" ? v : void 0;
        getDataViewMemory0().setBigInt64(arg0 + 8 * 1, isLikeNone(ret) ? BigInt(0) : ret, true);
        getDataViewMemory0().setInt32(arg0 + 4 * 0, !isLikeNone(ret), true);
      },
      __wbg___wbindgen_boolean_get_fa956cfa2d1bd751: function(arg0) {
        const v = arg0;
        const ret = typeof v === "boolean" ? v : void 0;
        return isLikeNone(ret) ? 16777215 : ret ? 1 : 0;
      },
      __wbg___wbindgen_debug_string_c25d447a39f5578f: function(arg0, arg1) {
        const ret = debugString(arg1);
        const ptr1 = passStringToWasm0(ret, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len1 = WASM_VECTOR_LEN;
        getDataViewMemory0().setInt32(arg0 + 4 * 1, len1, true);
        getDataViewMemory0().setInt32(arg0 + 4 * 0, ptr1, true);
      },
      __wbg___wbindgen_in_aca499c5de7ff5e5: function(arg0, arg1) {
        const ret = arg0 in arg1;
        return ret;
      },
      __wbg___wbindgen_is_bigint_2f76dc55065b4273: function(arg0) {
        const ret = typeof arg0 === "bigint";
        return ret;
      },
      __wbg___wbindgen_is_function_1ff95bcc5517c252: function(arg0) {
        const ret = typeof arg0 === "function";
        return ret;
      },
      __wbg___wbindgen_is_null_ea9085d691f535d3: function(arg0) {
        const ret = arg0 === null;
        return ret;
      },
      __wbg___wbindgen_is_object_a27215656b807791: function(arg0) {
        const val = arg0;
        const ret = typeof val === "object" && val !== null;
        return ret;
      },
      __wbg___wbindgen_is_string_ea5e6cc2e4141dfe: function(arg0) {
        const ret = typeof arg0 === "string";
        return ret;
      },
      __wbg___wbindgen_is_undefined_c05833b95a3cf397: function(arg0) {
        const ret = arg0 === void 0;
        return ret;
      },
      __wbg___wbindgen_jsval_eq_e659fcf7b0e32763: function(arg0, arg1) {
        const ret = arg0 === arg1;
        return ret;
      },
      __wbg___wbindgen_jsval_loose_eq_db4c3b15f63fc170: function(arg0, arg1) {
        const ret = arg0 == arg1;
        return ret;
      },
      __wbg___wbindgen_number_get_394265ed1e1b84ee: function(arg0, arg1) {
        const obj = arg1;
        const ret = typeof obj === "number" ? obj : void 0;
        getDataViewMemory0().setFloat64(arg0 + 8 * 1, isLikeNone(ret) ? 0 : ret, true);
        getDataViewMemory0().setInt32(arg0 + 4 * 0, !isLikeNone(ret), true);
      },
      __wbg___wbindgen_string_get_b0ca35b86a603356: function(arg0, arg1) {
        const obj = arg1;
        const ret = typeof obj === "string" ? obj : void 0;
        var ptr1 = isLikeNone(ret) ? 0 : passStringToWasm0(ret, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        var len1 = WASM_VECTOR_LEN;
        getDataViewMemory0().setInt32(arg0 + 4 * 1, len1, true);
        getDataViewMemory0().setInt32(arg0 + 4 * 0, ptr1, true);
      },
      __wbg___wbindgen_throw_344f42d3211c4765: function(arg0, arg1) {
        throw new Error(getStringFromWasm0(arg0, arg1));
      },
      __wbg__wbg_cb_unref_fffb441def202758: function(arg0) {
        arg0._wbg_cb_unref();
      },
      __wbg_abort_8a4b90d8b05efcf8: function() {
        return handleError(function(arg0) {
          arg0.abort();
        }, arguments);
      },
      __wbg_abort_8bae0f33e7833997: function(arg0) {
        arg0.abort();
      },
      __wbg_abort_eee9248a6d680839: function(arg0, arg1) {
        arg0.abort(arg1);
      },
      __wbg_append_01c74e5c6b58aa64: function() {
        return handleError(function(arg0, arg1, arg2, arg3, arg4) {
          arg0.append(getStringFromWasm0(arg1, arg2), getStringFromWasm0(arg3, arg4));
        }, arguments);
      },
      __wbg_apply_3ac86a26fdb56c05: function() {
        return handleError(function(arg0, arg1, arg2) {
          const ret = arg0.apply(arg1, arg2);
          return ret;
        }, arguments);
      },
      __wbg_arrayBuffer_3b637f0fa65c5351: function() {
        return handleError(function(arg0) {
          const ret = arg0.arrayBuffer();
          return ret;
        }, arguments);
      },
      __wbg_arrayBuffer_a158e423a87ee756: function(arg0) {
        const ret = arg0.arrayBuffer();
        return ret;
      },
      __wbg_blob_f072da1dc9650b5f: function() {
        return handleError(function(arg0) {
          const ret = arg0.blob();
          return ret;
        }, arguments);
      },
      __wbg_body_2a9122b81e23d7e2: function(arg0) {
        const ret = arg0.body;
        return isLikeNone(ret) ? 0 : addToExternrefTable0(ret);
      },
      __wbg_bound_696859c2d8dac8bf: function() {
        return handleError(function(arg0, arg1, arg2, arg3) {
          const ret = IDBKeyRange.bound(arg0, arg1, arg2 !== 0, arg3 !== 0);
          return ret;
        }, arguments);
      },
      __wbg_buffer_54b87055582c8a81: function(arg0) {
        const ret = arg0.buffer;
        return ret;
      },
      __wbg_byobRequest_06b654bb15590436: function(arg0) {
        const ret = arg0.byobRequest;
        return isLikeNone(ret) ? 0 : addToExternrefTable0(ret);
      },
      __wbg_byteLength_41862ca4020b9c43: function(arg0) {
        const ret = arg0.byteLength;
        return ret;
      },
      __wbg_byteOffset_d42e18c4441f628b: function(arg0) {
        const ret = arg0.byteOffset;
        return ret;
      },
      __wbg_cache_9948aa0322850557: function(arg0) {
        const ret = arg0.cache;
        return (__wbindgen_enum_RequestCache.indexOf(ret) + 1 || 7) - 1;
      },
      __wbg_caches_8dc528cff9e7c01b: function() {
        return handleError(function(arg0) {
          const ret = arg0.caches;
          return ret;
        }, arguments);
      },
      __wbg_call_44b7209e1e252e6a: function() {
        return handleError(function(arg0, arg1, arg2, arg3, arg4) {
          const ret = arg0.call(arg1, arg2, arg3, arg4);
          return ret;
        }, arguments);
      },
      __wbg_call_8a2dd23819f8a60a: function() {
        return handleError(function(arg0, arg1) {
          const ret = arg0.call(arg1);
          return ret;
        }, arguments);
      },
      __wbg_call_a6e5c5dce5018821: function() {
        return handleError(function(arg0, arg1, arg2) {
          const ret = arg0.call(arg1, arg2);
          return ret;
        }, arguments);
      },
      __wbg_call_e3b662382210db98: function() {
        return handleError(function(arg0, arg1, arg2, arg3) {
          const ret = arg0.call(arg1, arg2, arg3);
          return ret;
        }, arguments);
      },
      __wbg_cancel_3983a93e24cc66b3: function(arg0) {
        const ret = arg0.cancel();
        return ret;
      },
      __wbg_catch_c1a60df4c30d76d3: function(arg0, arg1) {
        const ret = arg0.catch(arg1);
        return ret;
      },
      __wbg_clearTimeout_6b8d9a38b9263d65: function(arg0) {
        const ret = clearTimeout(arg0);
        return ret;
      },
      __wbg_clientId_a8c15cda78f341ce: function(arg0, arg1) {
        const ret = arg1.clientId;
        var ptr1 = isLikeNone(ret) ? 0 : passStringToWasm0(ret, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        var len1 = WASM_VECTOR_LEN;
        getDataViewMemory0().setInt32(arg0 + 4 * 1, len1, true);
        getDataViewMemory0().setInt32(arg0 + 4 * 0, ptr1, true);
      },
      __wbg_clients_e342c77cd8bed164: function(arg0) {
        const ret = arg0.clients;
        return ret;
      },
      __wbg_close_175f8e039af34f61: function(arg0) {
        const ret = arg0.close();
        return ret;
      },
      __wbg_close_249a23304523681b: function() {
        return handleError(function(arg0) {
          arg0.close();
        }, arguments);
      },
      __wbg_close_463f5a813b1d0317: function(arg0) {
        arg0.close();
      },
      __wbg_close_4c3686e8e8c6d353: function(arg0) {
        arg0.close();
      },
      __wbg_close_72d318d9c16e83ef: function() {
        return handleError(function(arg0) {
          arg0.close();
        }, arguments);
      },
      __wbg_close_d00f4cb641f9db10: function(arg0) {
        arg0.close();
      },
      __wbg_contains_72b3d3ec2e94729e: function(arg0, arg1, arg2) {
        const ret = arg0.contains(getStringFromWasm0(arg1, arg2));
        return ret;
      },
      __wbg_createObjectStore_ff668af6e79f0433: function() {
        return handleError(function(arg0, arg1, arg2) {
          const ret = arg0.createObjectStore(getStringFromWasm0(arg1, arg2));
          return ret;
        }, arguments);
      },
      __wbg_createSyncAccessHandle_0caafebe31e4f2d9: function(arg0) {
        const ret = arg0.createSyncAccessHandle();
        return ret;
      },
      __wbg_createWritable_659288d5245034d3: function(arg0) {
        const ret = arg0.createWritable();
        return ret;
      },
      __wbg_crypto_29a13c24f21f7c72: function() {
        return handleError(function(arg0) {
          const ret = arg0.crypto;
          return ret;
        }, arguments);
      },
      __wbg_crypto_38df2bab126b63dc: function(arg0) {
        const ret = arg0.crypto;
        return ret;
      },
      __wbg_crypto_ef7cb54f00697d0c: function() {
        return handleError(function(arg0) {
          const ret = arg0.crypto;
          return ret;
        }, arguments);
      },
      __wbg_data_572fc643bb2fce1c: function(arg0) {
        const ret = arg0.data;
        return ret;
      },
      __wbg_decodeURIComponent_eeb2408cbb904737: function() {
        return handleError(function(arg0, arg1) {
          const ret = decodeURIComponent(getStringFromWasm0(arg0, arg1));
          return ret;
        }, arguments);
      },
      __wbg_decrypt_ee3bb3762428b573: function() {
        return handleError(function(arg0, arg1, arg2, arg3) {
          const ret = arg0.decrypt(arg1, arg2, arg3);
          return ret;
        }, arguments);
      },
      __wbg_delete_e7e50168de5ef96e: function() {
        return handleError(function(arg0, arg1) {
          const ret = arg0.delete(arg1);
          return ret;
        }, arguments);
      },
      __wbg_delete_f809d7e5c6e4bdf1: function(arg0, arg1, arg2) {
        const ret = arg0.delete(getStringFromWasm0(arg1, arg2));
        return ret;
      },
      __wbg_delete_space_storage_59958cc964ba1a39: function(arg0, arg1) {
        const ret = delete_space_storage(getStringFromWasm0(arg0, arg1));
        return ret;
      },
      __wbg_deriveBits_e5bd401e3a409572: function() {
        return handleError(function(arg0, arg1, arg2, arg3) {
          const ret = arg0.deriveBits(arg1, arg2, arg3 >>> 0);
          return ret;
        }, arguments);
      },
      __wbg_deriveKey_ea5de45ca2f3aac6: function() {
        return handleError(function(arg0, arg1, arg2, arg3, arg4, arg5) {
          const ret = arg0.deriveKey(arg1, arg2, arg3, arg4 !== 0, arg5);
          return ret;
        }, arguments);
      },
      __wbg_done_89b2b13e91a60321: function(arg0) {
        const ret = arg0.done;
        return ret;
      },
      __wbg_encrypt_d3eb710b5823d9dc: function() {
        return handleError(function(arg0, arg1, arg2, arg3) {
          const ret = arg0.encrypt(arg1, arg2, arg3);
          return ret;
        }, arguments);
      },
      __wbg_enqueue_6d83b4c6281bafd6: function() {
        return handleError(function(arg0, arg1) {
          arg0.enqueue(arg1);
        }, arguments);
      },
      __wbg_entries_015dc610cd81ede0: function(arg0) {
        const ret = Object.entries(arg0);
        return ret;
      },
      __wbg_entries_900cefd6f70eb290: function(arg0) {
        const ret = arg0.entries();
        return ret;
      },
      __wbg_error_5b02424faf301d7c: function(arg0) {
        const ret = arg0.error;
        return isLikeNone(ret) ? 0 : addToExternrefTable0(ret);
      },
      __wbg_error_becd7e1fe6ce0623: function() {
        return handleError(function(arg0) {
          const ret = arg0.error;
          return isLikeNone(ret) ? 0 : addToExternrefTable0(ret);
        }, arguments);
      },
      __wbg_exportKey_31e8893b69d9dc95: function() {
        return handleError(function(arg0, arg1, arg2, arg3) {
          const ret = arg0.exportKey(getStringFromWasm0(arg1, arg2), arg3);
          return ret;
        }, arguments);
      },
      __wbg_extractable_8b9ecad52c0b0b3b: function(arg0) {
        const ret = arg0.extractable;
        return ret;
      },
      __wbg_fetch_8682caccbccb001a: function(arg0) {
        const ret = fetch(arg0);
        return ret;
      },
      __wbg_fetch_9dad4fe911207b37: function(arg0) {
        const ret = fetch(arg0);
        return ret;
      },
      __wbg_fetch_b5951fc96f52f786: function(arg0, arg1) {
        const ret = arg0.fetch(arg1);
        return ret;
      },
      __wbg_fill_5663f5c647107cce: function(arg0, arg1, arg2, arg3) {
        const ret = arg0.fill(arg1, arg2 >>> 0, arg3 >>> 0);
        return ret;
      },
      __wbg_flush_a4bd8d4e05ad23f6: function() {
        return handleError(function(arg0) {
          arg0.flush();
        }, arguments);
      },
      __wbg_frameType_69891d34f6e0249e: function(arg0) {
        const ret = arg0.frameType;
        return (__wbindgen_enum_FrameType.indexOf(ret) + 1 || 5) - 1;
      },
      __wbg_from_13e323c65fc8f464: function(arg0) {
        const ret = Array.from(arg0);
        return ret;
      },
      __wbg_generateKey_bd3b6d19d7d161d8: function() {
        return handleError(function(arg0, arg1, arg2, arg3) {
          const ret = arg0.generateKey(arg1, arg2 !== 0, arg3);
          return ret;
        }, arguments);
      },
      __wbg_getAllKeys_600fd10abc7076d8: function() {
        return handleError(function(arg0) {
          const ret = arg0.getAllKeys();
          return ret;
        }, arguments);
      },
      __wbg_getAllKeys_a180741be38f0d7b: function() {
        return handleError(function(arg0, arg1) {
          const ret = arg0.getAllKeys(arg1);
          return ret;
        }, arguments);
      },
      __wbg_getAllKeys_fc0ea02394e29868: function() {
        return handleError(function(arg0, arg1, arg2) {
          const ret = arg0.getAllKeys(arg1, arg2 >>> 0);
          return ret;
        }, arguments);
      },
      __wbg_getDirectoryHandle_cf175faf1a75a384: function(arg0, arg1, arg2, arg3) {
        const ret = arg0.getDirectoryHandle(getStringFromWasm0(arg1, arg2), arg3);
        return ret;
      },
      __wbg_getDirectory_389283588dfb8117: function(arg0) {
        const ret = arg0.getDirectory();
        return ret;
      },
      __wbg_getFileHandle_72de55ab3ca9ad57: function(arg0, arg1, arg2, arg3) {
        const ret = arg0.getFileHandle(getStringFromWasm0(arg1, arg2), arg3);
        return ret;
      },
      __wbg_getFile_bdc0144baa662031: function(arg0) {
        const ret = arg0.getFile();
        return ret;
      },
      __wbg_getRandomValues_3f44b700395062e5: function() {
        return handleError(function(arg0, arg1) {
          globalThis.crypto.getRandomValues(getArrayU8FromWasm0(arg0, arg1));
        }, arguments);
      },
      __wbg_getRandomValues_c44a50d8cfdaebeb: function() {
        return handleError(function(arg0, arg1) {
          arg0.getRandomValues(arg1);
        }, arguments);
      },
      __wbg_getRandomValues_cc7f052a444bb2ce: function() {
        return handleError(function(arg0, arg1) {
          globalThis.crypto.getRandomValues(getArrayU8FromWasm0(arg0, arg1));
        }, arguments);
      },
      __wbg_getReader_bb0230851fbf986b: function() {
        return handleError(function(arg0) {
          const ret = arg0.getReader();
          return ret;
        }, arguments);
      },
      __wbg_get_4771b0fab98477d2: function(arg0, arg1, arg2) {
        const ret = arg1[arg2 >>> 0];
        var ptr1 = isLikeNone(ret) ? 0 : passStringToWasm0(ret, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        var len1 = WASM_VECTOR_LEN;
        getDataViewMemory0().setInt32(arg0 + 4 * 1, len1, true);
        getDataViewMemory0().setInt32(arg0 + 4 * 0, ptr1, true);
      },
      __wbg_get_507a50627bffa49b: function(arg0, arg1) {
        const ret = arg0[arg1 >>> 0];
        return ret;
      },
      __wbg_get_78f252d074a84d0b: function() {
        return handleError(function(arg0, arg1) {
          const ret = Reflect.get(arg0, arg1);
          return ret;
        }, arguments);
      },
      __wbg_get_c7eb1f358a7654df: function() {
        return handleError(function(arg0, arg1) {
          const ret = Reflect.get(arg0, arg1);
          return ret;
        }, arguments);
      },
      __wbg_get_cefddcaffca4fbb7: function() {
        return handleError(function(arg0, arg1) {
          const ret = arg0.get(arg1);
          return ret;
        }, arguments);
      },
      __wbg_get_done_670108eb06ecbe46: function(arg0) {
        const ret = arg0.done;
        return isLikeNone(ret) ? 16777215 : ret ? 1 : 0;
      },
      __wbg_get_f4654c7a03b1d8a1: function(arg0, arg1, arg2) {
        const ret = arg0.get(getStringFromWasm0(arg1, arg2));
        return ret;
      },
      __wbg_get_unchecked_6e0ad6d2a41b06f6: function(arg0, arg1) {
        const ret = arg0[arg1 >>> 0];
        return ret;
      },
      __wbg_get_value_f465f5be30aa0963: function(arg0) {
        const ret = arg0.value;
        return ret;
      },
      __wbg_has_8374cf06984d8bfc: function() {
        return handleError(function(arg0, arg1) {
          const ret = Reflect.has(arg0, arg1);
          return ret;
        }, arguments);
      },
      __wbg_headers_7b59c5203c8c475d: function(arg0) {
        const ret = arg0.headers;
        return ret;
      },
      __wbg_headers_cf9c80f30e2a4eff: function(arg0) {
        const ret = arg0.headers;
        return ret;
      },
      __wbg_id_20613c7a0e9be11d: function(arg0, arg1) {
        const ret = arg1.id;
        const ptr1 = passStringToWasm0(ret, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len1 = WASM_VECTOR_LEN;
        getDataViewMemory0().setInt32(arg0 + 4 * 1, len1, true);
        getDataViewMemory0().setInt32(arg0 + 4 * 0, ptr1, true);
      },
      __wbg_importKey_10a137b5a53c66ba: function() {
        return handleError(function(arg0, arg1, arg2, arg3, arg4, arg5, arg6, arg7) {
          const ret = arg0.importKey(getStringFromWasm0(arg1, arg2), arg3, getStringFromWasm0(arg4, arg5), arg6 !== 0, arg7);
          return ret;
        }, arguments);
      },
      __wbg_importKey_a897ceb72b8a829b: function() {
        return handleError(function(arg0, arg1, arg2, arg3, arg4, arg5, arg6) {
          const ret = arg0.importKey(getStringFromWasm0(arg1, arg2), arg3, arg4, arg5 !== 0, arg6);
          return ret;
        }, arguments);
      },
      __wbg_instanceof_ArrayBuffer_4480b9e0068a8adb: function(arg0) {
        let result;
        try {
          result = arg0 instanceof ArrayBuffer;
        } catch (_) {
          result = false;
        }
        const ret = result;
        return ret;
      },
      __wbg_instanceof_Blob_c6523f92a32c8695: function(arg0) {
        let result;
        try {
          result = arg0 instanceof Blob;
        } catch (_) {
          result = false;
        }
        const ret = result;
        return ret;
      },
      __wbg_instanceof_CacheStorage_88ff75f83a2546e7: function(arg0) {
        let result;
        try {
          result = arg0 instanceof CacheStorage;
        } catch (_) {
          result = false;
        }
        const ret = result;
        return ret;
      },
      __wbg_instanceof_Cache_86e5408252b10cb9: function(arg0) {
        let result;
        try {
          result = arg0 instanceof Cache;
        } catch (_) {
          result = false;
        }
        const ret = result;
        return ret;
      },
      __wbg_instanceof_Client_fb9be9f30916959d: function(arg0) {
        let result;
        try {
          result = arg0 instanceof Client;
        } catch (_) {
          result = false;
        }
        const ret = result;
        return ret;
      },
      __wbg_instanceof_CryptoKey_fe4356820192d6c1: function(arg0) {
        let result;
        try {
          result = arg0 instanceof CryptoKey;
        } catch (_) {
          result = false;
        }
        const ret = result;
        return ret;
      },
      __wbg_instanceof_DomException_952faa8037702c00: function(arg0) {
        let result;
        try {
          result = arg0 instanceof DOMException;
        } catch (_) {
          result = false;
        }
        const ret = result;
        return ret;
      },
      __wbg_instanceof_Error_1fdac9f13a8181ba: function(arg0) {
        let result;
        try {
          result = arg0 instanceof Error;
        } catch (_) {
          result = false;
        }
        const ret = result;
        return ret;
      },
      __wbg_instanceof_FileSystemDirectoryHandle_c9ab7c5cdb7a7c30: function(arg0) {
        let result;
        try {
          result = arg0 instanceof FileSystemDirectoryHandle;
        } catch (_) {
          result = false;
        }
        const ret = result;
        return ret;
      },
      __wbg_instanceof_FileSystemFileHandle_68e80b30532d5f04: function(arg0) {
        let result;
        try {
          result = arg0 instanceof FileSystemFileHandle;
        } catch (_) {
          result = false;
        }
        const ret = result;
        return ret;
      },
      __wbg_instanceof_FileSystemSyncAccessHandle_db0b7504516129c1: function(arg0) {
        let result;
        try {
          result = arg0 instanceof FileSystemSyncAccessHandle;
        } catch (_) {
          result = false;
        }
        const ret = result;
        return ret;
      },
      __wbg_instanceof_FileSystemWritableFileStream_bbd33ec1789b2714: function(arg0) {
        let result;
        try {
          result = arg0 instanceof FileSystemWritableFileStream;
        } catch (_) {
          result = false;
        }
        const ret = result;
        return ret;
      },
      __wbg_instanceof_File_ee62de53bca2e697: function(arg0) {
        let result;
        try {
          result = arg0 instanceof File;
        } catch (_) {
          result = false;
        }
        const ret = result;
        return ret;
      },
      __wbg_instanceof_IdbDatabase_1cc734ba1b040dd7: function(arg0) {
        let result;
        try {
          result = arg0 instanceof IDBDatabase;
        } catch (_) {
          result = false;
        }
        const ret = result;
        return ret;
      },
      __wbg_instanceof_IdbFactory_85ccbbe95ef25434: function(arg0) {
        let result;
        try {
          result = arg0 instanceof IDBFactory;
        } catch (_) {
          result = false;
        }
        const ret = result;
        return ret;
      },
      __wbg_instanceof_IdbRequest_fe7a4cb10800af5b: function(arg0) {
        let result;
        try {
          result = arg0 instanceof IDBRequest;
        } catch (_) {
          result = false;
        }
        const ret = result;
        return ret;
      },
      __wbg_instanceof_LockManager_aaf744196b58befc: function(arg0) {
        let result;
        try {
          result = arg0 instanceof LockManager;
        } catch (_) {
          result = false;
        }
        const ret = result;
        return ret;
      },
      __wbg_instanceof_Map_e5b5e3db98422fcc: function(arg0) {
        let result;
        try {
          result = arg0 instanceof Map;
        } catch (_) {
          result = false;
        }
        const ret = result;
        return ret;
      },
      __wbg_instanceof_MessagePort_b17bf77c564b57da: function(arg0) {
        let result;
        try {
          result = arg0 instanceof MessagePort;
        } catch (_) {
          result = false;
        }
        const ret = result;
        return ret;
      },
      __wbg_instanceof_Promise_4cb210c0b8f8c959: function(arg0) {
        let result;
        try {
          result = arg0 instanceof Promise;
        } catch (_) {
          result = false;
        }
        const ret = result;
        return ret;
      },
      __wbg_instanceof_ReadableStream_3bdc7d10b03fd402: function(arg0) {
        let result;
        try {
          result = arg0 instanceof ReadableStream;
        } catch (_) {
          result = false;
        }
        const ret = result;
        return ret;
      },
      __wbg_instanceof_Response_c8b64b2256f01bec: function(arg0) {
        let result;
        try {
          result = arg0 instanceof Response;
        } catch (_) {
          result = false;
        }
        const ret = result;
        return ret;
      },
      __wbg_instanceof_ServiceWorkerGlobalScope_cdda7caab46fb9f0: function(arg0) {
        let result;
        try {
          result = arg0 instanceof ServiceWorkerGlobalScope;
        } catch (_) {
          result = false;
        }
        const ret = result;
        return ret;
      },
      __wbg_instanceof_StorageManager_6f57a8d458090b58: function(arg0) {
        let result;
        try {
          result = arg0 instanceof StorageManager;
        } catch (_) {
          result = false;
        }
        const ret = result;
        return ret;
      },
      __wbg_instanceof_Uint8Array_309b927aaf7a3fc7: function(arg0) {
        let result;
        try {
          result = arg0 instanceof Uint8Array;
        } catch (_) {
          result = false;
        }
        const ret = result;
        return ret;
      },
      __wbg_instanceof_WindowClient_8229e3b796b4fc4b: function(arg0) {
        let result;
        try {
          result = arg0 instanceof WindowClient;
        } catch (_) {
          result = false;
        }
        const ret = result;
        return ret;
      },
      __wbg_instanceof_Window_05ba1ee4f6781663: function(arg0) {
        let result;
        try {
          result = arg0 instanceof Window;
        } catch (_) {
          result = false;
        }
        const ret = result;
        return ret;
      },
      __wbg_instanceof_WorkerGlobalScope_8ec07b5e040a41c3: function(arg0) {
        let result;
        try {
          result = arg0 instanceof WorkerGlobalScope;
        } catch (_) {
          result = false;
        }
        const ret = result;
        return ret;
      },
      __wbg_isArray_0677c962b281d01a: function(arg0) {
        const ret = Array.isArray(arg0);
        return ret;
      },
      __wbg_isSafeInteger_04f36e4056f1b851: function(arg0) {
        const ret = Number.isSafeInteger(arg0);
        return ret;
      },
      __wbg_iterator_6f722e4a93058b71: function() {
        const ret = Symbol.iterator;
        return ret;
      },
      __wbg_length_1f0964f4a5e2c6d8: function(arg0) {
        const ret = arg0.length;
        return ret;
      },
      __wbg_length_370319915dc99107: function(arg0) {
        const ret = arg0.length;
        return ret;
      },
      __wbg_length_e01fceeaca9c95fd: function(arg0) {
        const ret = arg0.length;
        return ret;
      },
      __wbg_location_10b37366f445bf01: function(arg0) {
        const ret = arg0.location;
        return ret;
      },
      __wbg_log_d267660666346fb3: function(arg0) {
        console.log(arg0);
      },
      __wbg_matchAll_1061db1eb945fdb3: function(arg0) {
        const ret = arg0.matchAll();
        return ret;
      },
      __wbg_matchAll_3720d78ef9b0f05e: function(arg0, arg1) {
        const ret = arg0.matchAll(arg1);
        return ret;
      },
      __wbg_match_5403e213aa78598b: function(arg0, arg1, arg2) {
        const ret = arg0.match(getStringFromWasm0(arg1, arg2));
        return ret;
      },
      __wbg_match_8f071926d6a643ec: function(arg0, arg1, arg2) {
        const ret = arg0.match(arg1, arg2);
        return ret;
      },
      __wbg_message_8326fb1d549bebc5: function(arg0) {
        const ret = arg0.message;
        return ret;
      },
      __wbg_message_fb0e6e7854e6ea7a: function(arg0, arg1) {
        const ret = arg1.message;
        const ptr1 = passStringToWasm0(ret, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len1 = WASM_VECTOR_LEN;
        getDataViewMemory0().setInt32(arg0 + 4 * 1, len1, true);
        getDataViewMemory0().setInt32(arg0 + 4 * 0, ptr1, true);
      },
      __wbg_method_b84982a2d750a05f: function(arg0, arg1) {
        const ret = arg1.method;
        const ptr1 = passStringToWasm0(ret, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len1 = WASM_VECTOR_LEN;
        getDataViewMemory0().setInt32(arg0 + 4 * 1, len1, true);
        getDataViewMemory0().setInt32(arg0 + 4 * 0, ptr1, true);
      },
      __wbg_mode_306d974410c79b0b: function(arg0) {
        const ret = arg0.mode;
        return (__wbindgen_enum_RequestMode.indexOf(ret) + 1 || 5) - 1;
      },
      __wbg_msCrypto_bd5a034af96bcba6: function(arg0) {
        const ret = arg0.msCrypto;
        return ret;
      },
      __wbg_name_9d2bcd24d4433cef: function(arg0, arg1) {
        const ret = arg1.name;
        const ptr1 = passStringToWasm0(ret, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len1 = WASM_VECTOR_LEN;
        getDataViewMemory0().setInt32(arg0 + 4 * 1, len1, true);
        getDataViewMemory0().setInt32(arg0 + 4 * 0, ptr1, true);
      },
      __wbg_name_b0b4809690944614: function(arg0) {
        const ret = arg0.name;
        return ret;
      },
      __wbg_navigator_51379c10a84aeec9: function(arg0) {
        const ret = arg0.navigator;
        return ret;
      },
      __wbg_new_0d809930cd1354c6: function() {
        return handleError(function() {
          const ret = new Headers();
          return ret;
        }, arguments);
      },
      __wbg_new_245322ac8cd2a4ae: function() {
        return handleError(function(arg0, arg1) {
          const ret = new BroadcastChannel(getStringFromWasm0(arg0, arg1));
          return ret;
        }, arguments);
      },
      __wbg_new_32b398fb48b6d94a: function() {
        const ret = new Array();
        return ret;
      },
      __wbg_new_4339b2a2675a03e3: function() {
        return handleError(function() {
          const ret = new AbortController();
          return ret;
        }, arguments);
      },
      __wbg_new_7796ffc7ed656783: function() {
        const ret = /* @__PURE__ */ new Map();
        return ret;
      },
      __wbg_new_aec3e25493d729fe: function(arg0, arg1) {
        try {
          var state0 = { a: arg0, b: arg1 };
          var cb0 = (arg02, arg12) => {
            const a = state0.a;
            state0.a = 0;
            try {
              return wasm_bindgen__convert__closures_____invoke__h20dfb9cdde521b79(a, state0.b, arg02, arg12);
            } finally {
              state0.a = a;
            }
          };
          const ret = new Promise(cb0);
          return ret;
        } finally {
          state0.a = 0;
        }
      },
      __wbg_new_b667d279fd5aa943: function(arg0, arg1) {
        const ret = new Error(getStringFromWasm0(arg0, arg1));
        return ret;
      },
      __wbg_new_cd45aabdf6073e84: function(arg0) {
        const ret = new Uint8Array(arg0);
        return ret;
      },
      __wbg_new_da52cf8fe3429cb2: function() {
        const ret = new Object();
        return ret;
      },
      __wbg_new_from_slice_77cdfb7977362f3c: function(arg0, arg1) {
        const ret = new Uint8Array(getArrayU8FromWasm0(arg0, arg1));
        return ret;
      },
      __wbg_new_typed_1824d93f294193e5: function(arg0, arg1) {
        try {
          var state0 = { a: arg0, b: arg1 };
          var cb0 = (arg02, arg12) => {
            const a = state0.a;
            state0.a = 0;
            try {
              return wasm_bindgen__convert__closures_____invoke__h20dfb9cdde521b79(a, state0.b, arg02, arg12);
            } finally {
              state0.a = a;
            }
          };
          const ret = new Promise(cb0);
          return ret;
        } finally {
          state0.a = 0;
        }
      },
      __wbg_new_with_byte_offset_and_length_54c7724ee3ec7d82: function(arg0, arg1, arg2) {
        const ret = new Uint8Array(arg0, arg1 >>> 0, arg2 >>> 0);
        return ret;
      },
      __wbg_new_with_into_underlying_source_8812003c9985c511: function(arg0, arg1) {
        const ret = new ReadableStream(IntoUnderlyingSource.__wrap(arg0), arg1);
        return ret;
      },
      __wbg_new_with_length_e6785c33c8e4cce8: function(arg0) {
        const ret = new Uint8Array(arg0 >>> 0);
        return ret;
      },
      __wbg_new_with_opt_readable_stream_and_init_bfd13e2b23ff2e0c: function() {
        return handleError(function(arg0, arg1) {
          const ret = new Response(arg0, arg1);
          return ret;
        }, arguments);
      },
      __wbg_new_with_opt_str_and_init_74529c37934bd6b4: function() {
        return handleError(function(arg0, arg1, arg2) {
          const ret = new Response(arg0 === 0 ? void 0 : getStringFromWasm0(arg0, arg1), arg2);
          return ret;
        }, arguments);
      },
      __wbg_new_with_opt_u8_array_14c47b6c15712e97: function() {
        return handleError(function(arg0, arg1) {
          const ret = new Response(arg0 === 0 ? void 0 : getArrayU8FromWasm0(arg0, arg1));
          return ret;
        }, arguments);
      },
      __wbg_new_with_str_54bc0f9c32770e1e: function() {
        return handleError(function(arg0, arg1) {
          const ret = new Request(getStringFromWasm0(arg0, arg1));
          return ret;
        }, arguments);
      },
      __wbg_new_with_str_and_init_d95cbe11ce28e65e: function() {
        return handleError(function(arg0, arg1, arg2) {
          const ret = new Request(getStringFromWasm0(arg0, arg1), arg2);
          return ret;
        }, arguments);
      },
      __wbg_next_6dbf2c0ac8cde20f: function(arg0) {
        const ret = arg0.next;
        return ret;
      },
      __wbg_next_71f2aa1cb3d1e37e: function() {
        return handleError(function(arg0) {
          const ret = arg0.next();
          return ret;
        }, arguments);
      },
      __wbg_node_84ea875411254db1: function(arg0) {
        const ret = arg0.node;
        return ret;
      },
      __wbg_now_86c0d4ba3fa605b8: function() {
        const ret = Date.now();
        return ret;
      },
      __wbg_now_e7c6795a7f81e10f: function(arg0) {
        const ret = arg0.now();
        return ret;
      },
      __wbg_objectStoreNames_146ab25540bff6db: function(arg0) {
        const ret = arg0.objectStoreNames;
        return ret;
      },
      __wbg_objectStore_d5f47956b6c741e3: function() {
        return handleError(function(arg0, arg1, arg2) {
          const ret = arg0.objectStore(getStringFromWasm0(arg1, arg2));
          return ret;
        }, arguments);
      },
      __wbg_of_2bf3ed8a776ff19a: function(arg0, arg1, arg2, arg3) {
        const ret = Array.of(arg0, arg1, arg2, arg3);
        return ret;
      },
      __wbg_ok_acc5e3fb89668864: function(arg0) {
        const ret = arg0.ok;
        return ret;
      },
      __wbg_onLine_e4b3d6727b94b166: function(arg0) {
        const ret = arg0.onLine;
        return ret;
      },
      __wbg_open_1ee80f2c655b05e9: function(arg0, arg1, arg2) {
        const ret = arg0.open(getStringFromWasm0(arg1, arg2));
        return ret;
      },
      __wbg_open_72e5234a49d5f85d: function() {
        return handleError(function(arg0, arg1, arg2, arg3) {
          const ret = arg0.open(getStringFromWasm0(arg1, arg2), arg3 >>> 0);
          return ret;
        }, arguments);
      },
      __wbg_open_8b445bd20535cb55: function() {
        return handleError(function(arg0, arg1, arg2) {
          const ret = arg0.open(getStringFromWasm0(arg1, arg2));
          return ret;
        }, arguments);
      },
      __wbg_origin_563ae02896bf4edc: function(arg0, arg1) {
        const ret = arg1.origin;
        const ptr1 = passStringToWasm0(ret, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len1 = WASM_VECTOR_LEN;
        getDataViewMemory0().setInt32(arg0 + 4 * 1, len1, true);
        getDataViewMemory0().setInt32(arg0 + 4 * 0, ptr1, true);
      },
      __wbg_parse_1c0d8a8656d7e016: function() {
        return handleError(function(arg0, arg1) {
          const ret = JSON.parse(getStringFromWasm0(arg0, arg1));
          return ret;
        }, arguments);
      },
      __wbg_performance_3fcf6e32a7e1ed0a: function(arg0) {
        const ret = arg0.performance;
        return ret;
      },
      __wbg_ports_ae5e4719d70644c8: function(arg0) {
        const ret = arg0.ports;
        return ret;
      },
      __wbg_postMessage_3d105cccfd888270: function() {
        return handleError(function(arg0, arg1) {
          arg0.postMessage(arg1);
        }, arguments);
      },
      __wbg_postMessage_b80f20949a4b4f55: function() {
        return handleError(function(arg0, arg1) {
          arg0.postMessage(arg1);
        }, arguments);
      },
      __wbg_postMessage_dd820b768b365613: function() {
        return handleError(function(arg0, arg1) {
          arg0.postMessage(arg1);
        }, arguments);
      },
      __wbg_process_44c7a14e11e9f69e: function(arg0) {
        const ret = arg0.process;
        return ret;
      },
      __wbg_prototypesetcall_4770620bbe4688a0: function(arg0, arg1, arg2) {
        Uint8Array.prototype.set.call(getArrayU8FromWasm0(arg0, arg1), arg2);
      },
      __wbg_push_d2ae3af0c1217ae6: function(arg0, arg1) {
        const ret = arg0.push(arg1);
        return ret;
      },
      __wbg_put_3067c7a6df4dec59: function() {
        return handleError(function(arg0, arg1) {
          const ret = arg0.put(arg1);
          return ret;
        }, arguments);
      },
      __wbg_put_650ce93c775fde08: function(arg0, arg1, arg2, arg3) {
        const ret = arg0.put(getStringFromWasm0(arg1, arg2), arg3);
        return ret;
      },
      __wbg_put_a368805e3dcab3a7: function() {
        return handleError(function(arg0, arg1, arg2) {
          const ret = arg0.put(arg1, arg2);
          return ret;
        }, arguments);
      },
      __wbg_queueMicrotask_0ab5b2d2393e99b9: function(arg0) {
        const ret = arg0.queueMicrotask;
        return ret;
      },
      __wbg_queueMicrotask_6a09b7bc46549209: function(arg0) {
        queueMicrotask(arg0);
      },
      __wbg_randomFillSync_6c25eac9869eb53c: function() {
        return handleError(function(arg0, arg1) {
          arg0.randomFillSync(arg1);
        }, arguments);
      },
      __wbg_read_8afa15f12a160ef8: function(arg0) {
        const ret = arg0.read();
        return ret;
      },
      __wbg_redirected_ef8aef1710a452df: function(arg0) {
        const ret = arg0.redirected;
        return ret;
      },
      __wbg_registration_eac835ac15600cdd: function(arg0) {
        const ret = arg0.registration;
        return ret;
      },
      __wbg_releaseLock_5b92874cad775644: function(arg0) {
        arg0.releaseLock();
      },
      __wbg_removeEntry_e38219fa4a98cfb3: function(arg0, arg1, arg2) {
        const ret = arg0.removeEntry(getStringFromWasm0(arg1, arg2));
        return ret;
      },
      __wbg_request_1bb1026ec312149e: function(arg0) {
        const ret = arg0.request;
        return ret;
      },
      __wbg_request_2a3fe2992deb6481: function(arg0, arg1, arg2, arg3) {
        const ret = arg0.request(getStringFromWasm0(arg1, arg2), arg3);
        return ret;
      },
      __wbg_require_b4edbdcf3e2a1ef0: function() {
        return handleError(function() {
          const ret = module.require;
          return ret;
        }, arguments);
      },
      __wbg_resolve_2191a4dfe481c25b: function(arg0) {
        const ret = Promise.resolve(arg0);
        return ret;
      },
      __wbg_respond_510e32df8aeb6817: function() {
        return handleError(function(arg0, arg1) {
          arg0.respond(arg1 >>> 0);
        }, arguments);
      },
      __wbg_result_2b1294a2bf8dc773: function() {
        return handleError(function(arg0) {
          const ret = arg0.result;
          return ret;
        }, arguments);
      },
      __wbg_run_5aa314612b150933: function(arg0, arg1, arg2) {
        try {
          var state0 = { a: arg1, b: arg2 };
          var cb0 = () => {
            const a = state0.a;
            state0.a = 0;
            try {
              return wasm_bindgen__convert__closures_____invoke__h8ab3768d4f978c64(a, state0.b);
            } finally {
              state0.a = a;
            }
          };
          const ret = arg0.run(cb0);
          return ret;
        } finally {
          state0.a = 0;
        }
      },
      __wbg_setTimeout_f757f00851f76c42: function(arg0, arg1) {
        const ret = setTimeout(arg0, arg1);
        return ret;
      },
      __wbg_set_0de9c62c23d04ad5: function() {
        return handleError(function(arg0, arg1, arg2, arg3, arg4) {
          arg0.set(getStringFromWasm0(arg1, arg2), getStringFromWasm0(arg3, arg4));
        }, arguments);
      },
      __wbg_set_4d7dd76f3dae2926: function(arg0, arg1, arg2) {
        arg0.set(getArrayU8FromWasm0(arg1, arg2));
      },
      __wbg_set_575dd786d51585f8: function(arg0, arg1, arg2) {
        const ret = arg0.set(arg1, arg2);
        return ret;
      },
      __wbg_set_6be42768c690e380: function(arg0, arg1, arg2) {
        arg0[arg1] = arg2;
      },
      __wbg_set_8535240470bf2500: function() {
        return handleError(function(arg0, arg1, arg2) {
          const ret = Reflect.set(arg0, arg1, arg2);
          return ret;
        }, arguments);
      },
      __wbg_set_8a16b38e4805b298: function(arg0, arg1, arg2) {
        arg0[arg1 >>> 0] = arg2;
      },
      __wbg_set_additional_data_18e1cb74f3f32864: function(arg0, arg1) {
        arg0.additionalData = arg1;
      },
      __wbg_set_at_674f6538cd77adef: function(arg0, arg1) {
        arg0.at = arg1;
      },
      __wbg_set_body_029f2d171e0a005f: function(arg0, arg1) {
        arg0.body = arg1;
      },
      __wbg_set_cache_b4a740b195c051f4: function(arg0, arg1) {
        arg0.cache = __wbindgen_enum_RequestCache[arg1];
      },
      __wbg_set_cache_name_d24a70dc0b6a4f14: function(arg0, arg1, arg2) {
        arg0.cacheName = getStringFromWasm0(arg1, arg2);
      },
      __wbg_set_create_a807a6e9ac628698: function(arg0, arg1) {
        arg0.create = arg1 !== 0;
      },
      __wbg_set_create_fa1dfa475fac91e9: function(arg0, arg1) {
        arg0.create = arg1 !== 0;
      },
      __wbg_set_credentials_bb34a40189e3b43b: function(arg0, arg1) {
        arg0.credentials = __wbindgen_enum_RequestCredentials[arg1];
      },
      __wbg_set_headers_4be66d6f175ce615: function(arg0, arg1) {
        arg0.headers = arg1;
      },
      __wbg_set_headers_9c61d123c3ee1f10: function(arg0, arg1) {
        arg0.headers = arg1;
      },
      __wbg_set_high_water_mark_44d043cd607dd13a: function(arg0, arg1) {
        arg0.highWaterMark = arg1;
      },
      __wbg_set_ignore_search_4dcf6a7482a79d5c: function(arg0, arg1) {
        arg0.ignoreSearch = arg1 !== 0;
      },
      __wbg_set_iv_3e6d5e431df58a93: function(arg0, arg1) {
        arg0.iv = arg1;
      },
      __wbg_set_method_5532d59b92d76467: function(arg0, arg1, arg2) {
        arg0.method = getStringFromWasm0(arg1, arg2);
      },
      __wbg_set_mode_66c79886ad78fc05: function(arg0, arg1) {
        arg0.mode = __wbindgen_enum_RequestMode[arg1];
      },
      __wbg_set_name_ca0910cc3b07587b: function(arg0, arg1, arg2) {
        arg0.name = getStringFromWasm0(arg1, arg2);
      },
      __wbg_set_onabort_e8ad31807de2db24: function(arg0, arg1) {
        arg0.onabort = arg1;
      },
      __wbg_set_oncomplete_e6abb66d0ad42731: function(arg0, arg1) {
        arg0.oncomplete = arg1;
      },
      __wbg_set_onerror_3488a474171ed56d: function(arg0, arg1) {
        arg0.onerror = arg1;
      },
      __wbg_set_onsuccess_cd0c3642a2873e66: function(arg0, arg1) {
        arg0.onsuccess = arg1;
      },
      __wbg_set_onupgradeneeded_7b2cf4ba1c57e655: function(arg0, arg1) {
        arg0.onupgradeneeded = arg1;
      },
      __wbg_set_onversionchange_c4d25c90ac386854: function(arg0, arg1) {
        arg0.onversionchange = arg1;
      },
      __wbg_set_private_key_530d0f76522944e5: function(arg0, arg1) {
        arg0.privateKey = arg1;
      },
      __wbg_set_public_key_bf90d2dc733147a4: function(arg0, arg1) {
        arg0.publicKey = arg1;
      },
      __wbg_set_redirect_badd73a0bcb765e3: function(arg0, arg1) {
        arg0.redirect = __wbindgen_enum_RequestRedirect[arg1];
      },
      __wbg_set_signal_c4ef8faddb4c1446: function(arg0, arg1) {
        arg0.signal = arg1;
      },
      __wbg_set_status_3593dfcb55e7ee3c: function(arg0, arg1) {
        arg0.status = arg1;
      },
      __wbg_set_type_904c2f0374079eca: function(arg0, arg1) {
        arg0.type = __wbindgen_enum_ClientType[arg1];
      },
      __wbg_sign_773a78ab7121b403: function() {
        return handleError(function(arg0, arg1, arg2, arg3) {
          const ret = arg0.sign(arg1, arg2, arg3);
          return ret;
        }, arguments);
      },
      __wbg_signal_dad7cb35193abd31: function(arg0) {
        const ret = arg0.signal;
        return ret;
      },
      __wbg_size_6304a694765921a9: function(arg0) {
        const ret = arg0.size;
        return ret;
      },
      __wbg_slice_7c7553e3b38b0ddb: function() {
        return handleError(function(arg0, arg1, arg2) {
          const ret = arg0.slice(arg1, arg2);
          return ret;
        }, arguments);
      },
      __wbg_source_9d659d033d70ee06: function(arg0) {
        const ret = arg0.source;
        return isLikeNone(ret) ? 0 : addToExternrefTable0(ret);
      },
      __wbg_space_storage_exists_3f3da3438b60ed0c: function() {
        return handleError(function(arg0, arg1) {
          const ret = space_storage_exists(getStringFromWasm0(arg0, arg1));
          return ret;
        }, arguments);
      },
      __wbg_startCustodyProgress_3417b19055abc060: function() {
        return handleError(function(arg0) {
          const ret = startCustodyProgress(arg0);
          return ret;
        }, arguments);
      },
      __wbg_static_accessor_CREATE_TASK_7ee0dd8bc83df5b2: function() {
        const ret = typeof console === "undefined" ? null : console?.createTask;
        return isLikeNone(ret) ? 0 : addToExternrefTable0(ret);
      },
      __wbg_static_accessor_GLOBAL_4ef717fb391d88b7: function() {
        const ret = typeof global === "undefined" ? null : global;
        return isLikeNone(ret) ? 0 : addToExternrefTable0(ret);
      },
      __wbg_static_accessor_GLOBAL_THIS_8d1badc68b5a74f4: function() {
        const ret = typeof globalThis === "undefined" ? null : globalThis;
        return isLikeNone(ret) ? 0 : addToExternrefTable0(ret);
      },
      __wbg_static_accessor_SELF_146583524fe1469b: function() {
        const ret = typeof self === "undefined" ? null : self;
        return isLikeNone(ret) ? 0 : addToExternrefTable0(ret);
      },
      __wbg_static_accessor_WINDOW_f2829a2234d7819e: function() {
        const ret = typeof window === "undefined" ? null : window;
        return isLikeNone(ret) ? 0 : addToExternrefTable0(ret);
      },
      __wbg_status_c45b3b9b3033184a: function(arg0) {
        const ret = arg0.status;
        return ret;
      },
      __wbg_stream_0ea97b74f081f92b: function(arg0) {
        const ret = arg0.stream();
        return ret;
      },
      __wbg_stringify_b54333f60f1e4dad: function() {
        return handleError(function(arg0) {
          const ret = JSON.stringify(arg0);
          return ret;
        }, arguments);
      },
      __wbg_subarray_3ed232c8a6baee09: function(arg0, arg1, arg2) {
        const ret = arg0.subarray(arg1 >>> 0, arg2 >>> 0);
        return ret;
      },
      __wbg_subtle_ed1dfcaede24af07: function(arg0) {
        const ret = arg0.subtle;
        return ret;
      },
      __wbg_target_e759594a8d965ed7: function(arg0) {
        const ret = arg0.target;
        return isLikeNone(ret) ? 0 : addToExternrefTable0(ret);
      },
      __wbg_text_d3a29f7525a132c3: function() {
        return handleError(function(arg0) {
          const ret = arg0.text();
          return ret;
        }, arguments);
      },
      __wbg_then_16d107c451e9905d: function(arg0, arg1, arg2) {
        const ret = arg0.then(arg1, arg2);
        return ret;
      },
      __wbg_then_6ec10ae38b3e92f7: function(arg0, arg1) {
        const ret = arg0.then(arg1);
        return ret;
      },
      __wbg_timeout_c971ded6d319fad2: function(arg0) {
        const ret = AbortSignal.timeout(arg0 >>> 0);
        return ret;
      },
      __wbg_tonkBundledAsset_ccc7db56334a2956: function() {
        return handleError(function(arg0, arg1) {
          const ret = tonkBundledAsset(getStringFromWasm0(arg0, arg1));
          return ret;
        }, arguments);
      },
      __wbg_tonkserviceworker_new: function(arg0) {
        const ret = TonkServiceWorker.__wrap(arg0);
        return ret;
      },
      __wbg_transaction_f212158274461f32: function() {
        return handleError(function(arg0, arg1, arg2) {
          const ret = arg0.transaction(arg1, __wbindgen_enum_IdbTransactionMode[arg2]);
          return ret;
        }, arguments);
      },
      __wbg_truncate_98a6032d23095328: function() {
        return handleError(function(arg0, arg1) {
          arg0.truncate(arg1);
        }, arguments);
      },
      __wbg_type_f9d1981b7f250cfe: function(arg0) {
        const ret = arg0.type;
        return (__wbindgen_enum_ResponseType.indexOf(ret) + 1 || 7) - 1;
      },
      __wbg_unwrapKey_4cb8242a57956c29: function() {
        return handleError(function(arg0, arg1, arg2, arg3, arg4, arg5, arg6, arg7, arg8, arg9, arg10) {
          const ret = arg0.unwrapKey(getStringFromWasm0(arg1, arg2), getArrayU8FromWasm0(arg3, arg4), arg5, getStringFromWasm0(arg6, arg7), arg8, arg9 !== 0, arg10);
          return ret;
        }, arguments);
      },
      __wbg_url_abdb8fb08377f8c0: function(arg0, arg1) {
        const ret = arg1.url;
        const ptr1 = passStringToWasm0(ret, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len1 = WASM_VECTOR_LEN;
        getDataViewMemory0().setInt32(arg0 + 4 * 1, len1, true);
        getDataViewMemory0().setInt32(arg0 + 4 * 0, ptr1, true);
      },
      __wbg_url_f6cd241d61f89b82: function(arg0, arg1) {
        const ret = arg1.url;
        const ptr1 = passStringToWasm0(ret, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len1 = WASM_VECTOR_LEN;
        getDataViewMemory0().setInt32(arg0 + 4 * 1, len1, true);
        getDataViewMemory0().setInt32(arg0 + 4 * 0, ptr1, true);
      },
      __wbg_userAgent_fc05e84c36de8392: function() {
        return handleError(function(arg0, arg1) {
          const ret = arg1.userAgent;
          const ptr1 = passStringToWasm0(ret, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
          const len1 = WASM_VECTOR_LEN;
          getDataViewMemory0().setInt32(arg0 + 4 * 1, len1, true);
          getDataViewMemory0().setInt32(arg0 + 4 * 0, ptr1, true);
        }, arguments);
      },
      __wbg_value_a5d5488a9589444a: function(arg0) {
        const ret = arg0.value;
        return ret;
      },
      __wbg_verify_9013f34c7e0e6416: function() {
        return handleError(function(arg0, arg1, arg2, arg3, arg4) {
          const ret = arg0.verify(arg1, arg2, arg3, arg4);
          return ret;
        }, arguments);
      },
      __wbg_version_955f7846e0eabcc3: function(arg0) {
        const ret = arg0.version;
        return ret;
      },
      __wbg_versions_276b2795b1c6a219: function(arg0) {
        const ret = arg0.versions;
        return ret;
      },
      __wbg_view_21f1d4a4f175dfa9: function(arg0) {
        const ret = arg0.view;
        return isLikeNone(ret) ? 0 : addToExternrefTable0(ret);
      },
      __wbg_visibilityState_2b278bb38937283e: function(arg0) {
        const ret = arg0.visibilityState;
        return (__wbindgen_enum_VisibilityState.indexOf(ret) + 1 || 3) - 1;
      },
      __wbg_waitUntil_d14dd2b66b8a139c: function() {
        return handleError(function(arg0, arg1) {
          arg0.waitUntil(arg1);
        }, arguments);
      },
      __wbg_waiting_96e8ee8b6ab8cf93: function(arg0) {
        const ret = arg0.waiting;
        return isLikeNone(ret) ? 0 : addToExternrefTable0(ret);
      },
      __wbg_wrapKey_52f63ef7a2cff351: function() {
        return handleError(function(arg0, arg1, arg2, arg3, arg4, arg5, arg6) {
          const ret = arg0.wrapKey(getStringFromWasm0(arg1, arg2), arg3, arg4, getStringFromWasm0(arg5, arg6));
          return ret;
        }, arguments);
      },
      __wbg_write_5000d1e9282930ef: function() {
        return handleError(function(arg0, arg1) {
          const ret = arg0.write(arg1);
          return ret;
        }, arguments);
      },
      __wbg_write_e557b5312ec23477: function() {
        return handleError(function(arg0, arg1, arg2, arg3) {
          const ret = arg0.write(getArrayU8FromWasm0(arg1, arg2), arg3);
          return ret;
        }, arguments);
      },
      __wbindgen_cast_0000000000000001: function(arg0, arg1) {
        const ret = makeMutClosure(arg0, arg1, wasm_bindgen__convert__closures_____invoke__h48a79f315ec9f091);
        return ret;
      },
      __wbindgen_cast_0000000000000002: function(arg0, arg1) {
        const ret = makeMutClosure(arg0, arg1, wasm_bindgen__convert__closures_____invoke__h39c3dbb2e9b2eccb);
        return ret;
      },
      __wbindgen_cast_0000000000000003: function(arg0, arg1) {
        const ret = makeMutClosure(arg0, arg1, wasm_bindgen__convert__closures_____invoke__h81b8c3b4d26a43c6);
        return ret;
      },
      __wbindgen_cast_0000000000000004: function(arg0, arg1) {
        const ret = makeMutClosure(arg0, arg1, wasm_bindgen__convert__closures_____invoke__h0c15ae96f7cd8476);
        return ret;
      },
      __wbindgen_cast_0000000000000005: function(arg0, arg1) {
        const ret = makeMutClosure(arg0, arg1, wasm_bindgen__convert__closures_____invoke__hf030fd419f76830b);
        return ret;
      },
      __wbindgen_cast_0000000000000006: function(arg0, arg1) {
        const ret = makeMutClosure(arg0, arg1, wasm_bindgen__convert__closures_____invoke__h10a17ef82ca80c66);
        return ret;
      },
      __wbindgen_cast_0000000000000007: function(arg0) {
        const ret = arg0;
        return ret;
      },
      __wbindgen_cast_0000000000000008: function(arg0) {
        const ret = arg0;
        return ret;
      },
      __wbindgen_cast_0000000000000009: function(arg0, arg1) {
        const ret = getArrayU8FromWasm0(arg0, arg1);
        return ret;
      },
      __wbindgen_cast_000000000000000a: function(arg0, arg1) {
        const ret = getStringFromWasm0(arg0, arg1);
        return ret;
      },
      __wbindgen_cast_000000000000000b: function(arg0) {
        const ret = BigInt.asUintN(64, arg0);
        return ret;
      },
      __wbindgen_init_externref_table: function() {
        const table = wasm.__wbindgen_externrefs;
        const offset = table.grow(4);
        table.set(0, void 0);
        table.set(offset + 0, void 0);
        table.set(offset + 1, null);
        table.set(offset + 2, true);
        table.set(offset + 3, false);
      }
    };
    return {
      __proto__: null,
      "./worker_bg.js": import0,
      "./snippets/tonk-worker-c0482e79796fa9a7/inline2.js": inline2_exports,
      "./snippets/tonk-worker-c0482e79796fa9a7/inline2.js": inline2_exports
    };
  }
  function wasm_bindgen__convert__closures_____invoke__hf030fd419f76830b(arg0, arg1) {
    wasm.wasm_bindgen__convert__closures_____invoke__hf030fd419f76830b(arg0, arg1);
  }
  function wasm_bindgen__convert__closures_____invoke__h10a17ef82ca80c66(arg0, arg1) {
    wasm.wasm_bindgen__convert__closures_____invoke__h10a17ef82ca80c66(arg0, arg1);
  }
  function wasm_bindgen__convert__closures_____invoke__h8ab3768d4f978c64(arg0, arg1) {
    const ret = wasm.wasm_bindgen__convert__closures_____invoke__h8ab3768d4f978c64(arg0, arg1);
    return ret !== 0;
  }
  function wasm_bindgen__convert__closures_____invoke__h39c3dbb2e9b2eccb(arg0, arg1, arg2) {
    wasm.wasm_bindgen__convert__closures_____invoke__h39c3dbb2e9b2eccb(arg0, arg1, arg2);
  }
  function wasm_bindgen__convert__closures_____invoke__h0c15ae96f7cd8476(arg0, arg1, arg2) {
    wasm.wasm_bindgen__convert__closures_____invoke__h0c15ae96f7cd8476(arg0, arg1, arg2);
  }
  function wasm_bindgen__convert__closures_____invoke__h48a79f315ec9f091(arg0, arg1, arg2) {
    const ret = wasm.wasm_bindgen__convert__closures_____invoke__h48a79f315ec9f091(arg0, arg1, arg2);
    return ret;
  }
  function wasm_bindgen__convert__closures_____invoke__h81b8c3b4d26a43c6(arg0, arg1, arg2) {
    const ret = wasm.wasm_bindgen__convert__closures_____invoke__h81b8c3b4d26a43c6(arg0, arg1, arg2);
    if (ret[1]) {
      throw takeFromExternrefTable0(ret[0]);
    }
  }
  function wasm_bindgen__convert__closures_____invoke__h20dfb9cdde521b79(arg0, arg1, arg2, arg3) {
    wasm.wasm_bindgen__convert__closures_____invoke__h20dfb9cdde521b79(arg0, arg1, arg2, arg3);
  }
  var __wbindgen_enum_ClientType = ["window", "worker", "sharedworker", "serviceworker", "all"];
  var __wbindgen_enum_FrameType = ["auxiliary", "top-level", "nested", "none"];
  var __wbindgen_enum_IdbTransactionMode = ["readonly", "readwrite", "versionchange", "readwriteflush", "cleanup"];
  var __wbindgen_enum_ReadableStreamType = ["bytes"];
  var __wbindgen_enum_RequestCache = ["default", "no-store", "reload", "no-cache", "force-cache", "only-if-cached"];
  var __wbindgen_enum_RequestCredentials = ["omit", "same-origin", "include"];
  var __wbindgen_enum_RequestMode = ["same-origin", "no-cors", "cors", "navigate"];
  var __wbindgen_enum_RequestRedirect = ["follow", "error", "manual"];
  var __wbindgen_enum_ResponseType = ["basic", "cors", "default", "error", "opaque", "opaqueredirect"];
  var __wbindgen_enum_VisibilityState = ["hidden", "visible"];
  var IntoUnderlyingByteSourceFinalization = typeof FinalizationRegistry === "undefined" ? { register: () => {
  }, unregister: () => {
  } } : new FinalizationRegistry((ptr) => wasm.__wbg_intounderlyingbytesource_free(ptr, 1));
  var IntoUnderlyingSinkFinalization = typeof FinalizationRegistry === "undefined" ? { register: () => {
  }, unregister: () => {
  } } : new FinalizationRegistry((ptr) => wasm.__wbg_intounderlyingsink_free(ptr, 1));
  var IntoUnderlyingSourceFinalization = typeof FinalizationRegistry === "undefined" ? { register: () => {
  }, unregister: () => {
  } } : new FinalizationRegistry((ptr) => wasm.__wbg_intounderlyingsource_free(ptr, 1));
  var TonkServiceWorkerFinalization = typeof FinalizationRegistry === "undefined" ? { register: () => {
  }, unregister: () => {
  } } : new FinalizationRegistry((ptr) => wasm.__wbg_tonkserviceworker_free(ptr, 1));
  function addToExternrefTable0(obj) {
    const idx = wasm.__externref_table_alloc();
    wasm.__wbindgen_externrefs.set(idx, obj);
    return idx;
  }
  var CLOSURE_DTORS = typeof FinalizationRegistry === "undefined" ? { register: () => {
  }, unregister: () => {
  } } : new FinalizationRegistry((state) => wasm.__wbindgen_destroy_closure(state.a, state.b));
  function debugString(val) {
    const type = typeof val;
    if (type == "number" || type == "boolean" || val == null) {
      return `${val}`;
    }
    if (type == "string") {
      return `"${val}"`;
    }
    if (type == "symbol") {
      const description = val.description;
      if (description == null) {
        return "Symbol";
      } else {
        return `Symbol(${description})`;
      }
    }
    if (type == "function") {
      const name = val.name;
      if (typeof name == "string" && name.length > 0) {
        return `Function(${name})`;
      } else {
        return "Function";
      }
    }
    if (Array.isArray(val)) {
      const length = val.length;
      let debug = "[";
      if (length > 0) {
        debug += debugString(val[0]);
      }
      for (let i = 1; i < length; i++) {
        debug += ", " + debugString(val[i]);
      }
      debug += "]";
      return debug;
    }
    const builtInMatches = /\[object ([^\]]+)\]/.exec(toString.call(val));
    let className;
    if (builtInMatches && builtInMatches.length > 1) {
      className = builtInMatches[1];
    } else {
      return toString.call(val);
    }
    if (className == "Object") {
      try {
        return "Object(" + JSON.stringify(val) + ")";
      } catch (_) {
        return "Object";
      }
    }
    if (val instanceof Error) {
      return `${val.name}: ${val.message}
${val.stack}`;
    }
    return className;
  }
  function getArrayU8FromWasm0(ptr, len) {
    ptr = ptr >>> 0;
    return getUint8ArrayMemory0().subarray(ptr / 1, ptr / 1 + len);
  }
  var cachedDataViewMemory0 = null;
  function getDataViewMemory0() {
    if (cachedDataViewMemory0 === null || cachedDataViewMemory0.buffer.detached === true || cachedDataViewMemory0.buffer.detached === void 0 && cachedDataViewMemory0.buffer !== wasm.memory.buffer) {
      cachedDataViewMemory0 = new DataView(wasm.memory.buffer);
    }
    return cachedDataViewMemory0;
  }
  function getStringFromWasm0(ptr, len) {
    return decodeText(ptr >>> 0, len);
  }
  var cachedUint8ArrayMemory0 = null;
  function getUint8ArrayMemory0() {
    if (cachedUint8ArrayMemory0 === null || cachedUint8ArrayMemory0.byteLength === 0) {
      cachedUint8ArrayMemory0 = new Uint8Array(wasm.memory.buffer);
    }
    return cachedUint8ArrayMemory0;
  }
  function handleError(f, args) {
    try {
      return f.apply(this, args);
    } catch (e) {
      const idx = addToExternrefTable0(e);
      wasm.__wbindgen_exn_store(idx);
    }
  }
  function isLikeNone(x) {
    return x === void 0 || x === null;
  }
  function makeMutClosure(arg0, arg1, f) {
    const state = { a: arg0, b: arg1, cnt: 1 };
    const real = (...args) => {
      state.cnt++;
      const a = state.a;
      state.a = 0;
      try {
        return f(a, state.b, ...args);
      } finally {
        state.a = a;
        real._wbg_cb_unref();
      }
    };
    real._wbg_cb_unref = () => {
      if (--state.cnt === 0) {
        wasm.__wbindgen_destroy_closure(state.a, state.b);
        state.a = 0;
        CLOSURE_DTORS.unregister(state);
      }
    };
    CLOSURE_DTORS.register(real, state, state);
    return real;
  }
  function passArrayJsValueToWasm0(array, malloc) {
    const ptr = malloc(array.length * 4, 4) >>> 0;
    for (let i = 0; i < array.length; i++) {
      const add = addToExternrefTable0(array[i]);
      getDataViewMemory0().setUint32(ptr + 4 * i, add, true);
    }
    WASM_VECTOR_LEN = array.length;
    return ptr;
  }
  function passStringToWasm0(arg, malloc, realloc) {
    if (realloc === void 0) {
      const buf = cachedTextEncoder.encode(arg);
      const ptr2 = malloc(buf.length, 1) >>> 0;
      getUint8ArrayMemory0().subarray(ptr2, ptr2 + buf.length).set(buf);
      WASM_VECTOR_LEN = buf.length;
      return ptr2;
    }
    let len = arg.length;
    let ptr = malloc(len, 1) >>> 0;
    const mem = getUint8ArrayMemory0();
    let offset = 0;
    for (; offset < len; offset++) {
      const code = arg.charCodeAt(offset);
      if (code > 127) break;
      mem[ptr + offset] = code;
    }
    if (offset !== len) {
      if (offset !== 0) {
        arg = arg.slice(offset);
      }
      ptr = realloc(ptr, len, len = offset + arg.length * 3, 1) >>> 0;
      const view = getUint8ArrayMemory0().subarray(ptr + offset, ptr + len);
      const ret = cachedTextEncoder.encodeInto(arg, view);
      offset += ret.written;
      ptr = realloc(ptr, len, offset, 1) >>> 0;
    }
    WASM_VECTOR_LEN = offset;
    return ptr;
  }
  function takeFromExternrefTable0(idx) {
    const value = wasm.__wbindgen_externrefs.get(idx);
    wasm.__externref_table_dealloc(idx);
    return value;
  }
  var cachedTextDecoder = new TextDecoder("utf-8", { ignoreBOM: true, fatal: true });
  cachedTextDecoder.decode();
  var MAX_SAFARI_DECODE_BYTES = 2146435072;
  var numBytesDecoded = 0;
  function decodeText(ptr, len) {
    numBytesDecoded += len;
    if (numBytesDecoded >= MAX_SAFARI_DECODE_BYTES) {
      cachedTextDecoder = new TextDecoder("utf-8", { ignoreBOM: true, fatal: true });
      cachedTextDecoder.decode();
      numBytesDecoded = len;
    }
    return cachedTextDecoder.decode(getUint8ArrayMemory0().subarray(ptr, ptr + len));
  }
  var cachedTextEncoder = new TextEncoder();
  if (!("encodeInto" in cachedTextEncoder)) {
    cachedTextEncoder.encodeInto = function(arg, view) {
      const buf = cachedTextEncoder.encode(arg);
      view.set(buf);
      return {
        read: arg.length,
        written: buf.length
      };
    };
  }
  var WASM_VECTOR_LEN = 0;
  var wasmModule;
  var wasmInstance;
  var wasm;
  function __wbg_finalize_init(instance, module2) {
    wasmInstance = instance;
    wasm = instance.exports;
    wasmModule = module2;
    cachedDataViewMemory0 = null;
    cachedUint8ArrayMemory0 = null;
    wasm.__wbindgen_start();
    return wasm;
  }
  async function __wbg_load(module2, imports) {
    if (typeof Response === "function" && module2 instanceof Response) {
      if (typeof WebAssembly.instantiateStreaming === "function") {
        try {
          return await WebAssembly.instantiateStreaming(module2, imports);
        } catch (e) {
          const validResponse = module2.ok && expectedResponseType(module2.type);
          if (validResponse && module2.headers.get("Content-Type") !== "application/wasm") {
            console.warn("`WebAssembly.instantiateStreaming` failed because your server does not serve Wasm with `application/wasm` MIME type. Falling back to `WebAssembly.instantiate` which is slower. Original error:\n", e);
          } else {
            throw e;
          }
        }
      }
      const bytes = await module2.arrayBuffer();
      return await WebAssembly.instantiate(bytes, imports);
    } else {
      const instance = await WebAssembly.instantiate(module2, imports);
      if (instance instanceof WebAssembly.Instance) {
        return { instance, module: module2 };
      } else {
        return instance;
      }
    }
    function expectedResponseType(type) {
      switch (type) {
        case "basic":
        case "cors":
        case "default":
          return true;
      }
      return false;
    }
  }
  async function __wbg_init(module_or_path) {
    if (wasm !== void 0) return wasm;
    if (module_or_path !== void 0) {
      if (Object.getPrototypeOf(module_or_path) === Object.prototype) {
        ({ module_or_path } = module_or_path);
      } else {
        console.warn("using deprecated parameters for the initialization function; pass a single object instead");
      }
    }
    if (module_or_path === void 0) {
      module_or_path = new URL("worker_bg.wasm", import_meta.url);
    }
    const imports = __wbg_get_imports();
    if (typeof module_or_path === "string" || typeof Request === "function" && module_or_path instanceof Request || typeof URL === "function" && module_or_path instanceof URL) {
      module_or_path = fetch(module_or_path);
    }
    const { instance, module: module2 } = await __wbg_load(await module_or_path, imports);
    return __wbg_finalize_init(instance, module2);
  }

  // release.js
  var WORKER_WASM_HASH = "4906f0475c5c34ac";
  var SHELL_CACHE = "tonk-space-shell";
  var RUNTIME_CACHE = "tonk-space-runtime";
  var WORKER_WASM_URL;
  var WORKER_WASM_KEY;
  var STATIC_PREFIXES = [
    "/guest/",
    "/styles-",
    "/images/",
    "/fonts/",
    "/tonk-code/",
    "/tonk-prose/",
    "/tonk-table/"
  ];
  var HASHED_NAME = /-[0-9a-f]{16}(?=\.)/;
  var ASSET_PATH = /^\/asset:([1-9A-HJ-NP-Za-km-z]+)$/;
  var PORT_TIMEOUT_MS = 5e3;
  var ACK_TIMEOUT_MS = 3e3;
  var PROTOCOL = 2;
  var PROFILE = self.location.hostname.split(".")[0].startsWith("profile");
  var SHELL_PATH = PROFILE ? "/profile.html" : "/space.html";
  var log = (...args) => console.log(PROFILE ? "[Profile Worker]" : "[Space Worker]", ...args);
  var LOG_RING_CAPACITY = 5e3;
  var logRing = [];
  var remember = (level, message) => {
    logRing.push({ t: Date.now(), level, message: message.slice(0, 2e3) });
    if (logRing.length > LOG_RING_CAPACITY) logRing.shift();
  };
  for (const level of ["log", "warn", "error"]) {
    const original = console[level].bind(console);
    console[level] = (...args) => {
      remember(
        level,
        args.map((arg) => {
          if (typeof arg === "string") return arg;
          try {
            return arg instanceof Error ? arg.stack || String(arg) : JSON.stringify(arg);
          } catch {
            return String(arg);
          }
        }).join(" ")
      );
      original(...args);
    };
  }
  var handlers = {};
  handlers.unhandledrejection = (event) => {
    remember("error", `unhandledrejection: ${event.reason?.stack || event.reason}`);
  };
  var workerHealth = {
    state: "idle",
    // idle | initializing | ok | failed
    // The hash this script checked the wasm it booted against: evidence
    // that the worker runs the wasm built with it.
    workerWasm: null,
    error: null,
    attempts: 0,
    lastAttemptAt: null,
    startedAt: Date.now()
  };
  async function healthResponse() {
    return new Response(
      JSON.stringify({
        site: PROFILE ? "profile" : "space",
        // Which space a space's worker holds, once it holds one: a page
        // of this origin opened on its own finds its way to the app by it.
        space: PROFILE ? void 0 : await heldSpace().catch(() => null),
        worker: workerHealth.state,
        workerWasm: workerHealth.workerWasm,
        error: workerHealth.error,
        attempts: workerHealth.attempts,
        lastAttemptAt: workerHealth.lastAttemptAt,
        startedAt: workerHealth.startedAt,
        // The whole ring: a diagnostic that keeps only the tail has
        // already lost the phase it was asked about.
        log: logRing.slice()
      }),
      { status: 200, headers: { "content-type": "application/json" } }
    );
  }
  handlers.install = (event) => {
    self.skipWaiting();
    event.waitUntil(
      Promise.all([
        caches.open(SHELL_CACHE).then((cache) => Promise.all([cache.add(SHELL_PATH), siteOrigins()])),
        pinWorkerWasm(),
        // A worker installing over another takes over what that one
        // held in memory, once it has written it down.
        self.registration.active ? expectOverlayHandoff() : null
      ])
    );
  };
  handlers.activate = (event) => {
    event.waitUntil(
      awaitOverlayHandoff().catch((error) => log("could not wait for what the last worker held:", error)).then(() => Promise.all([self.clients.claim(), dropOtherWorkerWasm()]))
    );
  };
  var OVERLAY_HANDOFF_CACHE = "TONK_OVERLAY_HANDOFF";
  var OVERLAY_HANDOFF_URL = "/__tonk/overlay-handoff";
  var OVERLAY_HANDOFF_PENDING_URL = "/__tonk/overlay-handoff-pending";
  var OVERLAY_HANDOFF_WAIT_MS = 1e3;
  async function expectOverlayHandoff() {
    const cache = await caches.open(OVERLAY_HANDOFF_CACHE);
    await cache.delete(OVERLAY_HANDOFF_URL);
    await cache.put(OVERLAY_HANDOFF_PENDING_URL, new Response(String(Date.now())));
  }
  async function awaitOverlayHandoff() {
    const stored = (url) => caches.match(url, { cacheName: OVERLAY_HANDOFF_CACHE });
    if (!await stored(OVERLAY_HANDOFF_PENDING_URL)) return;
    const deadline = Date.now() + OVERLAY_HANDOFF_WAIT_MS;
    while (!await stored(OVERLAY_HANDOFF_URL) && Date.now() < deadline) {
      await delay(25);
    }
    await (await caches.open(OVERLAY_HANDOFF_CACHE)).delete(OVERLAY_HANDOFF_PENDING_URL);
  }
  async function pinWorkerWasm() {
    const cache = await caches.open(RUNTIME_CACHE);
    if (await cache.match(WORKER_WASM_KEY)) return;
    const response = await fetch(WORKER_WASM_URL, { cache: "no-cache" });
    if (!response.ok) throw new Error(`worker wasm: ${response.status}`);
    const bytes = await response.arrayBuffer();
    if (!await isWorkerWasm(bytes)) {
      throw new Error(`the served worker wasm is not ${WORKER_WASM_HASH}`);
    }
    await cache.put(WORKER_WASM_KEY, wasmResponse(bytes));
  }
  async function dropOtherWorkerWasm() {
    const cache = await caches.open(RUNTIME_CACHE);
    for (const request of await cache.keys()) {
      const kept = new URL(request.url).pathname.endsWith("/worker_bg.wasm");
      if (kept && request.url !== WORKER_WASM_KEY) {
        await cache.delete(request);
      }
    }
  }
  async function workerWasm() {
    const cache = await caches.open(RUNTIME_CACHE);
    const held = await cache.match(WORKER_WASM_KEY);
    if (held) {
      const bytes2 = await held.arrayBuffer();
      if (await isWorkerWasm(bytes2)) return wasmResponse(bytes2);
      log("the kept wasm is not the one built with this worker; fetching it again");
      await cache.delete(WORKER_WASM_KEY);
    }
    const response = await fetch(WORKER_WASM_URL, { cache: "no-cache" });
    if (!response.ok) throw new Error(`worker wasm: ${response.status}`);
    const bytes = await response.arrayBuffer();
    if (!await isWorkerWasm(bytes)) throw new Error("the served wasm is not this worker's");
    await cache.put(WORKER_WASM_KEY, wasmResponse(bytes));
    return wasmResponse(bytes);
  }
  function wasmResponse(bytes) {
    return new Response(bytes, { headers: { "content-type": "application/wasm" } });
  }
  async function isWorkerWasm(bytes) {
    if (WORKER_WASM_HASH === "dev") return true;
    const digest = new Uint8Array(await crypto.subtle.digest("SHA-256", bytes));
    const hex = [...digest].map((byte) => byte.toString(16).padStart(2, "0")).join("");
    return hex.startsWith(WORKER_WASM_HASH);
  }
  async function serveStatic(request) {
    const url = new URL(request.url);
    const cache = await caches.open(RUNTIME_CACHE);
    const hashed = HASHED_NAME.test(url.pathname);
    if (hashed) {
      const held = await cache.match(url.href);
      if (held) return held;
    }
    let response;
    try {
      response = await fetch(request);
    } catch (error) {
      const held = await cache.match(url.href);
      if (held) return held;
      throw error;
    }
    if (response.ok) {
      await cache.put(url.href, response.clone());
      if (hashed) await dropSuperseded(cache, url);
    }
    return response;
  }
  async function dropSuperseded(cache, url) {
    const stem = url.pathname.replace(HASHED_NAME, "");
    for (const request of await cache.keys()) {
      const other = new URL(request.url);
      if (other.href !== url.href && other.pathname.replace(HASHED_NAME, "") === stem) {
        await cache.delete(request);
      }
    }
  }
  var hostPort = null;
  var portWaiters = [];
  var nextId = 1;
  var pending = /* @__PURE__ */ new Map();
  var admitted = /* @__PURE__ */ new Map();
  var ADMIT_MS = 5e3;
  function admit(url) {
    const now = Date.now();
    for (const [held, until] of admitted) if (until <= now) admitted.delete(held);
    admitted.set(url, now + ADMIT_MS);
  }
  function takeAdmission(url) {
    const until = admitted.get(url);
    admitted.delete(url);
    return until !== void 0 && until > Date.now();
  }
  handlers.message = (event) => {
    const type = event.data?.type;
    if (type === "protocol") {
      event.ports[0]?.postMessage({ protocol: PROTOCOL });
      return;
    }
    if (type === "flush") {
      event.waitUntil(flushSession());
      return;
    }
    if (type === "claim") {
      event.waitUntil(self.clients.claim());
      return;
    }
    if (type === "content") {
      admit(event.data.url);
      event.ports[0]?.postMessage({ admitted: true });
      return;
    }
    if (type === "keepalive") return;
    if (type === "space-port" && PROFILE) {
      bindSpacePort(event.ports[0], event.data);
      return;
    }
    if ((type === "custody" || type === "ceremony-refused") && PROFILE) {
      event.waitUntil(messaged(event.data, event.ports, event.source?.id ?? ""));
      return;
    }
    if (type !== "port") return;
    const [port] = event.ports;
    if (!port) return;
    adopt(port, event.source?.id ?? "");
    event.waitUntil(
      siteWorker().catch((error) => log("the database failed to start:", error))
    );
  };
  function adopt(port, frame) {
    port.onmessage = (event) => {
      const { data } = event;
      if (typeof data?.ping === "number") port.postMessage({ pong: data.ping });
      else if (typeof data?.call === "number") answer(port, data, frame);
      else if (typeof data?.ask === "number") asked(port, data);
      else if (typeof data?.cancel === "number") answering.get(data.cancel)?.cancel().catch(() => {
      });
      else if (typeof data?.signal === "string") signalled(data.signal);
      else if (data?.message !== void 0) messaged(data.message, event.ports, frame);
      else onReply(event);
    };
    hostPort = port;
    for (const resolve of portWaiters.splice(0)) resolve(port);
    log("adopted a port up");
  }
  var answering = /* @__PURE__ */ new Map();
  async function answer(port, { call, request }, frame) {
    try {
      const response = await api({
        request: new Request(new URL(request.path, self.location.origin), {
          method: request.method,
          headers: request.headers,
          body: request.body ?? void 0
        }),
        clientId: frame,
        resultingClientId: "",
        waitUntil() {
        }
      });
      port.postMessage({ call, head: { status: response.status, headers: [...response.headers] } });
      if (response.body) {
        const reader = response.body.getReader();
        answering.set(call, reader);
        for (; ; ) {
          const { done, value } = await reader.read();
          if (done) break;
          const chunk = value.buffer.slice(value.byteOffset, value.byteOffset + value.byteLength);
          port.postMessage({ call, chunk }, [chunk]);
        }
      }
      port.postMessage({ call, end: true });
    } catch (error) {
      port.postMessage({ call, error: String(error?.message ?? error) });
    } finally {
      answering.delete(call);
      if (request.method !== "GET") sessionChanged();
    }
  }
  async function asked(port, { ask, snapshot }) {
    try {
      if (PROFILE || snapshot !== true) throw new Error("unknown request");
      const worker = await siteWorker();
      const taken = await worker.snapshotSpace(await heldSpace());
      if (!taken) {
        port.postMessage({ answer: ask, empty: true });
        return;
      }
      const content = taken.content.buffer;
      const revision = taken.revision.buffer;
      port.postMessage({ answer: ask, content, revision }, [content, revision]);
    } catch (error) {
      port.postMessage({ answer: ask, error: String(error?.message ?? error) });
    }
  }
  async function signalled(signal) {
    try {
      const worker = await siteWorker();
      if (signal === "grant") {
        await ensureGrant(worker, { renew: true });
        return;
      }
      if (signal === "forget" && !PROFILE) {
        await forgetSite();
        return;
      }
      for (const space of spacePorts.values()) space.port.postMessage({ signal });
      if (signal === "connectivity") await worker.onconnectivity?.();
      else if (signal === "visibility") await worker.onvisibility?.();
    } catch (error) {
      log(`could not act on ${signal}:`, error);
    }
  }
  async function messaged(data, ports, frame) {
    try {
      const worker = await siteWorker();
      const source = frame ? await self.clients.get(frame) ?? null : null;
      await worker.onmessage({ data, ports: [...ports], source });
    } catch (error) {
      log("could not deliver a message:", error);
    }
  }
  async function api(event) {
    const worker = await siteWorker();
    if (PROFILE) {
      const space = spaceOf(event.request);
      if (space && space !== spaceKey(await worker.profileDid())) return askSpace(event.request, space);
    }
    return worker.onfetch(event);
  }
  var SPACE_PATH = /^\/api\/(?:inspect\/)?repository\/((?:did:key:)?z[1-9A-HJ-NP-Za-km-z]+)\/(?:branch|remote|archive)\//;
  var SPACE_PORT_WAIT_MS = 1e4;
  var SPACE_PORT_ASK_EVERY_MS = 1500;
  var SPACE_ANSWER_WAIT_MS = 3e4;
  var SPACE_PROBE_WAIT_MS = 3e3;
  var SPACE_PROBE_EVERY_MS = 1e4;
  var SPACE_IDLE_MS = 6e4;
  var spacePorts = /* @__PURE__ */ new Map();
  var spacePortWaiters = /* @__PURE__ */ new Map();
  var nextSpaceCall = 1;
  var nextSpaceProbe = 1;
  var spaceKey = (repo) => repo.replace(/^did:key:/, "");
  function bindSpacePort(port, { repo, branch }) {
    if (!port || typeof repo !== "string" || typeof branch !== "string") return;
    const space = holdSpacePort(repo, port);
    port.onmessage = async ({ data }) => {
      if (typeof data?.pong === "number") {
        space.probes.get(data.pong)?.();
        return;
      }
      if (typeof data?.answer === "number") {
        space.asks.get(data.answer)?.(data);
        return;
      }
      if (typeof data?.call === "number") {
        spaceAnswered(space, data);
        return;
      }
      const id = data?.id;
      if (typeof id !== "number") return;
      port.postMessage({ id, ack: true });
      try {
        const worker = await siteWorker();
        if (typeof data.delegate === "string") {
          const grant = await worker.delegateSpace(repo, data.delegate);
          const chain = grant.chain.buffer;
          port.postMessage({ id, space: repo, chain, expires: grant.expires }, [chain]);
          return;
        }
        if (typeof data.invite === "number") {
          await worker.inviteSpace(repo, data.invite);
          port.postMessage({ id, invited: true });
          return;
        }
        if (data.authority === true) {
          port.postMessage({ id, authority: await worker.spaceAuthority(repo) });
          return;
        }
        if (data.revoke) {
          const receipt = await worker.revokeGrant(repo, JSON.stringify(data.revoke));
          port.postMessage({ id, receipt: JSON.parse(receipt) });
          return;
        }
        if (data.terms === true) {
          const terms = await worker.spaceTerms(repo);
          port.postMessage({ id, remote: terms.remote, account: terms.account, name: terms.name });
          if (data.holds === true) releaseSpace(worker, repo, true);
          return;
        }
        if (data.seeded === true) {
          await worker.settleSeed(repo);
          port.postMessage({ id, settled: true });
          releaseSpace(worker, repo, data.from === "remote");
          return;
        }
        if (data.seed === true) {
          const fresh = await worker.pendingSeed(repo);
          if (!fresh) {
            port.postMessage({ id, none: true });
            return;
          }
          const source = JSON.parse(fresh).copy_from;
          if (!source) {
            port.postMessage({ id, fresh });
            return;
          }
          const copy = await askSpaceWorker(spaceKey(source), { snapshot: true });
          if (copy.empty) throw new Error("the space to copy is empty");
          port.postMessage(
            { id, fresh, copy: { content: copy.content, revision: copy.revision } },
            [copy.content, copy.revision]
          );
          return;
        }
        if (data.snapshot === true) {
          const snapshot = await worker.snapshotSpace(repo);
          if (!snapshot) {
            port.postMessage({ id, empty: true });
            return;
          }
          const content = snapshot.content.buffer;
          const revision = snapshot.revision.buffer;
          port.postMessage({ id, content, revision }, [content, revision]);
          return;
        }
        throw new Error("unknown request");
      } catch (error) {
        port.postMessage({ id, error: String(error?.message ?? error) });
      }
    };
  }
  function holdSpacePort(repo, port) {
    const key = spaceKey(repo);
    const space = {
      key,
      port,
      calls: /* @__PURE__ */ new Map(),
      asks: /* @__PURE__ */ new Map(),
      probes: /* @__PURE__ */ new Map(),
      probing: false,
      lost: false,
      idle: null
    };
    const stale = spacePorts.get(key);
    if (stale) {
      clearTimeout(stale.idle);
      for (const [call, passed] of [...stale.calls]) {
        if (!repeatable(passed.request)) {
          passed.space = stale;
          passed.fail(new Error("the space's worker restarted"));
          continue;
        }
        passed.space = space;
        space.calls.set(call, passed);
        port.postMessage({ call, request: passed.request });
      }
      if (space.calls.size > 0) {
        log(`asking ${space.calls.size} thing(s) of ${key} again of its new worker`);
      }
      stale.calls.clear();
    }
    spacePorts.set(key, space);
    for (const resolve of spacePortWaiters.get(key) ?? []) resolve(space);
    spacePortWaiters.delete(key);
    watchSpace(space);
    restSpace(space);
    return space;
  }
  function repeatable({ method, path }) {
    return method === "GET" || method === "HEAD" || /\/(query|claim\/select)(\?|$)/.test(path);
  }
  function spacePort(key) {
    const held = spacePorts.get(key);
    if (held && !held.lost) return Promise.resolve(held);
    return new Promise((resolve) => {
      const asking = setInterval(() => askForSpacePort(key), SPACE_PORT_ASK_EVERY_MS);
      const settle = (value) => {
        clearInterval(asking);
        resolve(value);
      };
      const waiters = spacePortWaiters.get(key) ?? [];
      waiters.push(settle);
      spacePortWaiters.set(key, waiters);
      setTimeout(() => settle(null), SPACE_PORT_WAIT_MS);
      askForSpacePort(key);
    });
  }
  async function profilePages() {
    const pages = await self.clients.matchAll({ type: "window" });
    return pages.filter((page) => !page.url.endsWith("#connector"));
  }
  async function askForSpacePort(key) {
    for (const page of await profilePages()) {
      page.postMessage({ type: "need-space-port", repo: `did:key:${key}`, branch: "main" });
    }
  }
  function probeSpace(space) {
    const probe = nextSpaceProbe++;
    return new Promise((resolve) => {
      const timer = setTimeout(() => {
        space.probes.delete(probe);
        resolve(false);
      }, SPACE_PROBE_WAIT_MS);
      space.probes.set(probe, () => {
        clearTimeout(timer);
        space.probes.delete(probe);
        resolve(true);
      });
      space.port.postMessage({ ping: probe });
    });
  }
  async function watchSpace(space) {
    if (space.probing) return;
    space.probing = true;
    try {
      while (spacePorts.get(space.key) === space && space.calls.size > 0) {
        if (!await probeSpace(space)) {
          if (spacePorts.get(space.key) !== space) return;
          log(`the worker of ${space.key} went quiet; asking for a new port`);
          space.lost = true;
          await askForSpacePort(space.key);
          return;
        }
        await delay(SPACE_PROBE_EVERY_MS);
      }
    } finally {
      space.probing = false;
    }
  }
  function restSpace(space) {
    clearTimeout(space.idle);
    if (space.calls.size > 0) return;
    space.idle = setTimeout(async () => {
      if (spacePorts.get(space.key) !== space || space.calls.size > 0) return;
      spacePorts.delete(space.key);
      for (const page of await profilePages()) {
        page.postMessage({ type: "release-space-port", repo: `did:key:${space.key}`, branch: "main" });
      }
    }, SPACE_IDLE_MS);
  }
  async function askSpaceWorker(key, request) {
    const held = await spacePort(key);
    if (!held) throw new Error("the space's worker could not be reached");
    const ask = nextSpaceCall++;
    clearTimeout(held.idle);
    try {
      return await within(
        new Promise((resolve, reject) => {
          held.asks.set(ask, (data) => data.error ? reject(new Error(data.error)) : resolve(data));
          held.port.postMessage({ ...request, ask });
        }),
        SPACE_ANSWER_WAIT_MS,
        "the space's worker did not answer"
      );
    } finally {
      held.asks.delete(ask);
      restSpace(held);
    }
  }
  function spaceOf(request) {
    const match = SPACE_PATH.exec(new URL(request.url).pathname.replace(/%3A/gi, ":"));
    return match ? spaceKey(match[1]) : null;
  }
  function spaceUnreachable(message) {
    return new Response(JSON.stringify({ error: { kind: "space-unreachable", message } }), {
      status: 503,
      headers: { "content-type": "application/json", "retry-after": "2" }
    });
  }
  async function askSpace(request, key) {
    const held = await spacePort(key);
    if (!held) return spaceUnreachable("the space's worker could not be reached");
    const url = new URL(request.url);
    const body = request.method === "GET" || request.method === "HEAD" ? null : await request.arrayBuffer();
    const call = nextSpaceCall++;
    let controller;
    let answered;
    const head = new Promise((resolve, reject) => {
      answered = { resolve, reject };
    });
    const settled = () => {
      passed.space.calls.delete(call);
      restSpace(passed.space);
    };
    const passed = {
      // The port may be replaced while this is answered: `space` follows
      // the request to wherever it has moved.
      space: held,
      request: {
        method: request.method,
        path: url.pathname + url.search,
        headers: [...request.headers],
        body
      },
      // Asked again of a new worker, a request already answering keeps
      // the status it gave.
      head: answered.resolve,
      chunk: (bytes) => controller.enqueue(new Uint8Array(bytes)),
      end() {
        settled();
        controller.close();
      },
      fail(error) {
        settled();
        answered.reject(error);
        try {
          controller.error(error);
        } catch {
        }
      }
    };
    const drop = () => {
      if (!passed.space.calls.has(call)) return;
      passed.space.port.postMessage({ cancel: call });
      settled();
    };
    const stream = new ReadableStream({
      start(started) {
        controller = started;
      },
      cancel: drop
    });
    clearTimeout(held.idle);
    held.calls.set(call, passed);
    request.signal?.addEventListener("abort", drop);
    held.port.postMessage({ call, request: passed.request });
    watchSpace(held);
    const timer = setTimeout(
      () => answered.reject(new Error("the space's worker did not answer")),
      SPACE_ANSWER_WAIT_MS
    );
    try {
      const { status, headers } = await head;
      const bodiless = status === 204 || status === 304;
      if (bodiless) settled();
      return new Response(bodiless ? null : stream, { status, headers });
    } catch (error) {
      log(`${key} did not answer:`, error);
      drop();
      return spaceUnreachable(String(error?.message ?? error));
    } finally {
      clearTimeout(timer);
    }
  }
  if (PROFILE) {
    self.tonkAskSpace = async (space, method, path, body) => {
      const response = await askSpace(
        new Request(new URL(path, self.location.origin), {
          method,
          headers: body == null ? {} : { "content-type": "application/json" },
          body: body ?? void 0
        }),
        spaceKey(space)
      );
      return { status: response.status, body: await response.text() };
    };
    self.tonkSubscribeSpace = async (space, path, body) => {
      const response = await askSpace(
        new Request(new URL(path, self.location.origin), {
          method: "POST",
          headers: { "content-type": "application/json", accept: "text/event-stream" },
          body
        }),
        spaceKey(space)
      );
      return { status: response.status, body: response.body };
    };
    self.tonkForgetSpace = async (space) => {
      const key = spaceKey(space);
      const held = await spacePort(key);
      if (!held) throw new Error("the space's worker could not be reached");
      held.port.postMessage({ signal: "forget" });
      clearTimeout(held.idle);
      spacePorts.delete(key);
      for (const page of await profilePages()) {
        page.postMessage({ type: "release-space-port", repo: `did:key:${key}`, branch: "main" });
      }
    };
    self.tonkSpaceChanged = (space) => {
      for (const held of spacePorts.values()) {
        if (space == null || held.key === spaceKey(space)) held.port.postMessage({ signal: "grant" });
      }
    };
  }
  function spaceAnswered(space, data) {
    const call = space.calls.get(data.call);
    if (!call) return;
    if (data.head) call.head(data.head);
    else if (data.chunk) call.chunk(data.chunk);
    else if (data.end) call.end();
    else if (data.error) call.fail(new Error(data.error));
  }
  var releasing = /* @__PURE__ */ new Map();
  function releaseSpace(worker, repo, pushFirst) {
    const released = (releasing.get(repo) ?? Promise.resolve()).then(() => worker.releaseSpace(repo, pushFirst)).catch((error) => log(`still holding a copy of ${repo}:`, error));
    releasing.set(repo, released);
    return released;
  }
  function releaseSpaceReads(reason) {
    for (const space of spacePorts.values()) {
      for (const [call, passed] of [...space.calls]) {
        space.port.postMessage({ cancel: call });
        passed.fail(new Error(reason));
      }
    }
  }
  self.tonkBundledAsset = async (path) => {
    if (typeof path !== "string" || !path.startsWith("/library/") || path.includes("..") || path.includes("?") || path.includes("#")) {
      throw new Error("invalid bundled library path");
    }
    return fetch(new URL(path, self.location.origin));
  };
  var GRANT_KEY = "/__space/grant";
  var GRANT_VERSION = 5;
  var RENEW_MARGIN_SECONDS = 60 * 60;
  if (!PROFILE) self.tonkAskProfile = (request) => askHost(request);
  async function forgetSite() {
    log("forgetting this space");
    retired = true;
    for (const name of await caches.keys()) await caches.delete(name);
    for (const { name } of await indexedDB.databases()) indexedDB.deleteDatabase(name);
    const root = await navigator.storage.getDirectory();
    for await (const name of root.keys()) await root.removeEntry(name, { recursive: true });
    await self.registration.unregister();
  }
  var rust;
  function siteWorker() {
    if (rust) return rust;
    workerHealth.state = "initializing";
    workerHealth.attempts += 1;
    workerHealth.lastAttemptAt = Date.now();
    rust = Promise.resolve().then(async () => {
      self.tonkAppOrigin = (await siteOrigins())?.app;
    }).then(() => __wbg_init({ module_or_path: workerWasm() })).then(() => activate(WORKER_WASM_HASH, [], !PROFILE)).then(async (worker) => {
      if (!PROFILE) {
        const grant = await ensureGrant(worker);
        await worker.resumeSpace(grant.space);
        keepTerms(worker).catch((error) => log("could not check the space's terms:", error));
      }
      await restoreSession(worker);
      if (!PROFILE && !hostPort) portToHost().catch(() => {
      });
      workerHealth.state = "ok";
      workerHealth.workerWasm = WORKER_WASM_HASH;
      workerHealth.error = null;
      return worker;
    }).catch((error) => {
      rust = null;
      workerHealth.state = "failed";
      workerHealth.workerWasm = null;
      workerHealth.error = String(error?.message || error).slice(0, 2e3);
      throw error;
    });
    return rust;
  }
  async function heldGrant() {
    const held = await caches.match(GRANT_KEY, { cacheName: SHELL_CACHE });
    return held ? held.json() : null;
  }
  async function heldSpace() {
    return (await heldGrant())?.space ?? null;
  }
  var renewing = null;
  function renewIfDue() {
    if (PROFILE) return Promise.resolve();
    renewing ??= siteWorker().then(ensureGrant).catch((error) => log("could not renew the space's delegation:", error)).finally(() => {
      renewing = null;
    });
    return renewing;
  }
  async function keepTerms(worker) {
    const held = await heldGrant();
    if (!held) return;
    const terms = await askHost({ terms: true, holds: held.seeded === true });
    const same = (terms.remote ?? null) === (held.remote ?? null) && terms.account === held.account && (terms.name ?? null) === (held.name ?? null);
    if (same) {
      await nameMember(worker, held);
      return;
    }
    log("what the profile holds for this space changed; taking up a new delegation");
    await ensureGrant(worker, { renew: true });
  }
  async function nameMember(worker, held) {
    if (!held.name) return;
    await worker.nameMember(held.space, held.name).catch((error) => log("the roster was not named:", error));
  }
  var granting = Promise.resolve();
  function ensureGrant(worker, options) {
    const granted = granting.then(() => takeGrant(worker, options));
    granting = granted.catch(() => {
    });
    return granted;
  }
  async function tell(stage) {
    const pages = await self.clients.matchAll({ type: "window", includeUncontrolled: true });
    for (const page of pages) page.postMessage({ type: "status", stage });
  }
  async function takeGrant(worker, options) {
    const held = await heldGrant();
    const now = Date.now() / 1e3;
    const renew = options?.renew ?? false;
    if (!renew && held?.version === GRANT_VERSION && held.expires - now > RENEW_MARGIN_SECONDS) {
      return held;
    }
    if (held?.seeded) return bringSpace(worker, held);
    await tell("asking for the space");
    try {
      return await bringSpace(worker, held);
    } finally {
      await tell("ready");
    }
  }
  async function bringSpace(worker, held) {
    const audience = await worker.profileDid();
    const grant = await askHost({ delegate: audience });
    const terms = await worker.adoptSpace(grant.space, new Uint8Array(grant.chain));
    if (!held?.seeded) {
      await tell("replicating the space");
      const seed = await askHost({ seed: true });
      let from = "seed";
      if (seed.fresh && seed.copy) {
        await worker.createCopy(
          grant.space,
          seed.fresh,
          new Uint8Array(seed.copy.content),
          new Uint8Array(seed.copy.revision)
        );
      } else if (seed.fresh) {
        await worker.createContent(grant.space, seed.fresh);
      } else {
        from = "snapshot";
        const snapshot = await askHost({ snapshot: true }).catch((error) => {
          if (!terms.remote) throw error;
          log("the profile has no copy to start from; the space will fill as it syncs:", error);
          from = "remote";
          return { empty: true };
        });
        if (!snapshot.empty) {
          await worker.seedSpace(
            grant.space,
            new Uint8Array(snapshot.content),
            new Uint8Array(snapshot.revision)
          );
        }
      }
      await askHost({ seeded: true, from });
    }
    const record = {
      space: grant.space,
      expires: grant.expires,
      remote: terms.remote,
      account: terms.account,
      name: terms.name,
      seeded: true,
      version: GRANT_VERSION
    };
    const cache = await caches.open(SHELL_CACHE);
    await cache.put(GRANT_KEY, new Response(JSON.stringify(record)));
    await nameMember(worker, record);
    log(`holding a delegation for ${grant.space} until ${new Date(grant.expires * 1e3).toISOString()}`);
    return record;
  }
  function onReply({ data }) {
    const entry = pending.get(data?.id);
    if (!entry) return;
    if (data.ack) {
      entry.acked();
      return;
    }
    pending.delete(data.id);
    if (data.error) entry.reject(new Error(data.error));
    else entry.resolve(data);
  }
  async function portToHost() {
    if (hostPort) return hostPort;
    const brokered = new Promise((resolve) => portWaiters.push(resolve));
    const clients = await self.clients.matchAll({ type: "window" });
    if (clients.length === 0) {
      throw new Error("no page to broker a port up");
    }
    for (const client of clients) client.postMessage({ type: "need-port" });
    return within(brokered, PORT_TIMEOUT_MS, "no client brokered a port in time");
  }
  async function askHost(request) {
    for (let attempt = 0; ; attempt++) {
      const port = await portToHost();
      const id = nextId++;
      let acked;
      const ack = new Promise((resolve) => acked = resolve);
      const reply = new Promise((resolve, reject) => {
        pending.set(id, { resolve, reject, acked });
      });
      port.postMessage({ ...request, id });
      try {
        await within(ack, ACK_TIMEOUT_MS, "the worker above did not answer");
      } catch (error) {
        pending.delete(id);
        if (hostPort === port) hostPort = null;
        if (attempt > 0) throw error;
        log("the port up went quiet; asking for a new one");
        continue;
      }
      return reply;
    }
  }
  function within(promise, ms, message) {
    let timer;
    const timeout = new Promise((_, reject) => {
      timer = setTimeout(() => reject(new Error(message)), ms);
    });
    return Promise.race([promise, timeout]).finally(() => clearTimeout(timer));
  }
  var SESSION_KEY = "/__space/session";
  var SAVE_DELAY_MS = 1e3;
  var dirty = false;
  var saving = null;
  var saved = null;
  async function restoreSession(worker) {
    const held = await caches.match(SESSION_KEY, { cacheName: SHELL_CACHE });
    if (!held) return;
    const bytes = new Uint8Array(await held.arrayBuffer());
    const live = (await self.clients.matchAll({ type: "window" })).map((client) => client.id);
    if (await worker.restoreSession(bytes, live)) {
      saved = bytes;
      log("restored the saved session");
    }
  }
  function sessionChanged() {
    dirty = true;
    saving ??= delay(SAVE_DELAY_MS).then(saveWhileDirty).finally(() => {
      saving = null;
    });
    return saving;
  }
  async function flushSession() {
    if (!rust) return;
    dirty = true;
    await Promise.all([saving, saveWhileDirty()]);
  }
  async function saveWhileDirty() {
    while (dirty && rust) {
      dirty = false;
      try {
        const worker = await rust;
        const bytes = await worker.savedSession();
        if (saved && sameBytes(bytes, saved)) continue;
        const cache = await caches.open(SHELL_CACHE);
        await cache.put(SESSION_KEY, new Response(bytes));
        saved = bytes;
      } catch (error) {
        log("could not save the session:", error);
      }
    }
  }
  function sameBytes(a, b) {
    return a.length === b.length && a.every((byte, index) => byte === b[index]);
  }
  function delay(ms) {
    return new Promise((resolve) => setTimeout(resolve, ms));
  }
  function serveChanging(event) {
    const work = [];
    const view = {
      request: event.request,
      clientId: event.clientId,
      resultingClientId: event.resultingClientId,
      waitUntil(promise) {
        work.push(promise);
        event.waitUntil(promise);
      }
    };
    const response = api(view);
    event.waitUntil(
      response.then(() => Promise.allSettled(work)).then(sessionChanged, () => {
      })
    );
    return response;
  }
  var retired = false;
  function isActiveIncumbent() {
    return self.registration.active === self.serviceWorker;
  }
  async function retire(reason) {
    if (retired || !rust || !isActiveIncumbent()) return;
    retired = true;
    log(`handing over: ${reason}`);
    try {
      const worker = await rust;
      await flushSession();
      await worker.onupdatefound?.();
      releaseSpaceReads("this worker is retiring");
    } catch (error) {
      retired = false;
      log("failed to release streams:", error);
    }
  }
  function watchSuccessor(candidate) {
    if (!candidate || !isActiveIncumbent()) return;
    const observe = () => {
      if (["installed", "activating", "activated"].includes(candidate.state)) {
        candidate.removeEventListener("statechange", observe);
        retire("a newer worker installed");
      } else if (candidate.state === "redundant") {
        candidate.removeEventListener("statechange", observe);
      }
    };
    candidate.addEventListener("statechange", observe);
    observe();
  }
  var lookingForSuccessor = null;
  function lookForSuccessor() {
    if (retired) return Promise.resolve();
    lookingForSuccessor ??= self.registration.update().catch((error) => {
      log("could not look for a newer worker:", error);
    }).finally(() => {
      lookingForSuccessor = null;
    });
    return lookingForSuccessor;
  }
  function watchRegistration() {
    if (self.registration.waiting) {
      Promise.resolve().then(() => retire("a successor was already waiting at startup"));
    }
    watchSuccessor(self.registration.installing);
    self.registration.addEventListener("updatefound", () => {
      watchSuccessor(self.registration.installing);
    });
  }
  var CONFIG_PATH = "/.well-known/tonk";
  var sites;
  async function siteOrigins() {
    if (sites) return sites;
    const cache = await caches.open(SHELL_CACHE);
    try {
      const response = await fetch(CONFIG_PATH, { cache: "no-cache" });
      if (response.ok) await cache.put(CONFIG_PATH, response.clone());
      sites = (await response.json()).sites;
    } catch {
      sites = (await (await cache.match(CONFIG_PATH))?.json())?.sites;
    }
    return sites;
  }
  var NO_PASSKEYS = "publickey-credentials-get=(), publickey-credentials-create=()";
  function spacePolicy(sites2, { framedBySelf = false } = {}) {
    const scheme = sites2 ? new URL(sites2.app).protocol : null;
    const profile = sites2 ? `${scheme}//profile${sites2.suffix ?? ""}.${sites2.host}` : null;
    const outer = sites2 ? `${sites2.app} ${profile}` : "'none'";
    const ancestors = framedBySelf && sites2 ? `'self' ${outer}` : outer;
    const framed = sites2 ? ` ${scheme}//*.${sites2.host}` : "";
    const published = PROFILE ? " https: http://localhost:* http://127.0.0.1:*" : "";
    return [
      "default-src 'none'",
      "script-src 'self' blob: 'unsafe-inline' 'unsafe-eval' 'wasm-unsafe-eval'",
      "style-src 'self' blob: 'unsafe-inline'",
      `img-src 'self' blob: data:${published}`,
      "media-src 'self' blob:",
      "font-src 'self' data:",
      `connect-src 'self' blob: data:${published}`,
      `frame-src 'self' blob:${framed}`,
      "worker-src blob:",
      "form-action 'none'",
      "base-uri 'self'",
      `frame-ancestors ${ancestors}`
    ].join("; ");
  }
  async function serveShell() {
    let response;
    try {
      response = await fetch(SHELL_PATH, { cache: "no-cache" });
      if (response.ok) {
        const cache = await caches.open(SHELL_CACHE);
        await cache.put(SHELL_PATH, response.clone());
      }
    } catch {
      response = await caches.match(SHELL_PATH);
    }
    if (!response) return new Response("offline", { status: 503 });
    const headers = new Headers(response.headers);
    headers.set(
      "content-security-policy",
      spacePolicy(await siteOrigins(), { framedBySelf: true })
    );
    headers.set("permissions-policy", NO_PASSKEYS);
    headers.set("x-content-type-options", "nosniff");
    const shell = (await response.text()).replace("<head>", `<head>
        ${SHELL_MARK}`);
    headers.delete("content-length");
    return new Response(shell, { status: response.status, headers });
  }
  var SHELL_MARK = '<meta name="tonk-shell" content="worker" />';
  async function readAsset(hash) {
    const space = await heldSpace();
    if (!space) return { status: 404, headers: [], body: new TextEncoder().encode("not found").buffer };
    const worker = await siteWorker();
    const request = new Request(
      new URL(`/api/repository/${space}/branch/main/blob/asset:${hash}`, self.location.origin)
    );
    const response = await worker.onfetch({
      request,
      clientId: "",
      resultingClientId: "",
      waitUntil() {
      }
    });
    return { status: response.status, headers: [...response.headers], body: await response.arrayBuffer() };
  }
  async function storeAsset(event) {
    const space = await heldSpace();
    if (!space) return new Response("not found", { status: 404 });
    const worker = await siteWorker();
    const headers = new Headers();
    for (const name of ["content-type", "x-tonk-blob-name"]) {
      const value = event.request.headers.get(name);
      if (value) headers.set(name, value);
    }
    const request = new Request(
      new URL(`/api/repository/${space}/branch/main/blob`, self.location.origin),
      { method: "POST", headers, body: await event.request.arrayBuffer() }
    );
    const response = await worker.onfetch({
      request,
      clientId: event.clientId,
      resultingClientId: "",
      waitUntil(work) {
        event.waitUntil(work);
      }
    });
    if (!response.ok) return response;
    const stored = await response.json();
    return new Response(JSON.stringify(stored), {
      status: 201,
      headers: { "content-type": "application/json", location: `/${stored.entity}` }
    });
  }
  async function serveAsset(hash, request) {
    const reply = await readAsset(hash);
    const headers = new Headers(reply.headers);
    headers.set("x-content-type-options", "nosniff");
    if (request.mode === "navigate") {
      if (reply.status === 404) return serveShell();
      headers.set(
        "content-security-policy",
        spacePolicy(await siteOrigins(), { framedBySelf: true })
      );
      headers.set("permissions-policy", NO_PASSKEYS);
    }
    if (reply.status !== 200) {
      return new Response(reply.body, { status: reply.status, headers });
    }
    headers.set("accept-ranges", "bytes");
    const range = parseRange(request.headers.get("range"), reply.body.byteLength);
    if (!range) {
      headers.set("content-length", String(reply.body.byteLength));
      return new Response(request.method === "HEAD" ? null : reply.body, { headers });
    }
    const [start, end] = range;
    headers.set("content-range", `bytes ${start}-${end}/${reply.body.byteLength}`);
    headers.set("content-length", String(end - start + 1));
    const body = request.method === "HEAD" ? null : reply.body.slice(start, end + 1);
    return new Response(body, { status: 206, headers });
  }
  async function readRoute(pathname) {
    if (PROFILE) return null;
    const space = await heldSpace();
    if (!space) return null;
    const worker = await siteWorker();
    const response = await worker.onfetch({
      request: new Request(
        new URL(`/api/repository/${space}/branch/main/http${pathname}`, self.location.origin)
      ),
      clientId: "",
      resultingClientId: "",
      waitUntil() {
      }
    });
    return response.status === 200 ? response : null;
  }
  async function serveRoute(request, otherwise) {
    let reply = null;
    try {
      reply = await readRoute(new URL(request.url).pathname);
    } catch (error) {
      log("route read failed:", error);
    }
    if (!reply) return otherwise();
    const headers = new Headers(reply.headers);
    headers.set("x-content-type-options", "nosniff");
    if (!headers.has("cache-control")) headers.set("cache-control", "no-cache");
    if (request.mode === "navigate") {
      headers.set(
        "content-security-policy",
        spacePolicy(await siteOrigins(), { framedBySelf: true })
      );
      headers.set("permissions-policy", NO_PASSKEYS);
    }
    return new Response(request.method === "HEAD" ? null : reply.body, { headers });
  }
  function parseRange(header, size) {
    const match = /^bytes=(\d*)-(\d*)$/.exec(header ?? "");
    if (!match || size === 0) return null;
    const [, from, to] = match;
    if (from === "" && to === "") return null;
    if (from === "") {
      const suffix = Math.min(Number(to), size);
      return [size - suffix, size - 1];
    }
    const start = Number(from);
    if (start >= size) return null;
    const end = to === "" ? size - 1 : Math.min(Number(to), size - 1);
    return end < start ? null : [start, end];
  }
  function pageMayAsk(request, path) {
    if (path === "/api/language-server" || path === "/api/site" || path === "/api/sync") return true;
    const match = /^\/api\/(inspect\/)?repository\/[^/]+(\/.*)?$/.exec(path);
    if (!match) return false;
    const [, inspect, rest] = match;
    if (inspect || rest === void 0) return request.method === "GET";
    return rest.startsWith("/branch/");
  }
  var missing = () => new Response("not found", { status: 404 });
  var ICON_PATH = /^\/webawesome\/icons\/[a-z0-9-]+\/[a-z0-9-]+\.svg$/;
  var ICON_CACHE = "tonk-site-icons";
  async function serveIcon(request) {
    const cache = await caches.open(ICON_CACHE);
    const kept = await cache.match(request.url);
    if (kept) return kept;
    let fetched;
    try {
      fetched = await fetch(request.url);
    } catch (error) {
      return new Response(String(error?.message ?? error), { status: 502 });
    }
    if (fetched.ok) await cache.put(request.url, fetched.clone());
    return fetched;
  }
  handlers.fetch = (event) => {
    const url = new URL(event.request.url);
    if (url.origin !== self.location.origin) return;
    const asset = ASSET_PATH.exec(url.pathname);
    if (event.request.mode === "navigate" && !asset) {
      event.respondWith(
        takeAdmission(url.href) ? serveRoute(event.request, missing) : serveShell()
      );
      event.waitUntil(lookForSuccessor());
      return;
    }
    if (asset && (event.request.method === "GET" || event.request.method === "HEAD")) {
      event.respondWith(
        serveAsset(asset[1], event.request).catch((error) => {
          log("asset read failed:", error);
          return new Response(String(error.message), { status: 502 });
        })
      );
      return;
    }
    if (ICON_PATH.test(url.pathname) && event.request.method === "GET") {
      event.respondWith(serveIcon(event.request));
      return;
    }
    if (url.pathname === "/api/health" && (event.request.method === "GET" || event.request.method === "HEAD")) {
      event.respondWith(healthResponse());
      if (self.registration.waiting) {
        event.waitUntil(retire("a successor is waiting"));
      }
      return;
    }
    if (url.pathname.startsWith("/api/")) {
      if (!PROFILE && !pageMayAsk(event.request, url.pathname)) {
        event.respondWith(
          new Response(
            JSON.stringify({
              error: { kind: "forbidden", message: "not a space's page's to ask" }
            }),
            { status: 403, headers: { "content-type": "application/json" } }
          )
        );
        return;
      }
      event.respondWith(event.request.method === "GET" ? api(event) : serveChanging(event));
      event.waitUntil(renewIfDue());
      if (self.registration.waiting) {
        event.waitUntil(retire("a successor is waiting"));
      }
      return;
    }
    if (url.pathname === "/" && event.request.method === "PUT") {
      event.respondWith(
        storeAsset(event).catch((error) => {
          log("asset store failed:", error);
          return new Response(String(error.message), { status: 502 });
        })
      );
      return;
    }
    if (url.pathname === SHELL_PATH) return;
    if (event.request.method === "GET" && STATIC_PREFIXES.some((prefix) => url.pathname.startsWith(prefix))) {
      event.respondWith(serveStatic(event.request));
      return;
    }
    if (event.request.method === "GET" || event.request.method === "HEAD") {
      event.respondWith(serveRoute(event.request, missing));
      return;
    }
    event.respondWith(missing());
  };
  self.release = {
    manifest: {
      v: 1,
      port: PROTOCOL,
      wasm: { path: "./worker_bg.wasm", hash: WORKER_WASM_HASH }
    },
    start({ base }) {
      WORKER_WASM_URL = new URL("./worker_bg.wasm", base).href;
      WORKER_WASM_KEY = `${WORKER_WASM_URL}?${WORKER_WASM_HASH}`;
      watchRegistration();
      return handlers;
    }
  };
})();
