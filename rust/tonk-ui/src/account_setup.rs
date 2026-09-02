//! Account-creation side of the service-worker update-safety contract.
//!
//! The boot script and service worker consume one origin-global durable hold.
//! This module is its sole producer: it compares the page and controlling
//! worker while holding the shared Web Lock, commits the hold before WebAuthn,
//! and removes that exact hold only after the worker-mediated ceremony settles
//! safely. Broadcasts and DOM events are wakeups; IndexedDB under the lock is
//! the authority.

use wasm_bindgen::JsValue;

use crate::custody_relay::CeremonyError;

const LEASED_REVISION: &str = "0";

/// The exact durable hold this attempt owns.
#[derive(Debug)]
pub(crate) struct Lease {
    operation_id: String,
}

impl Lease {
    fn new() -> Self {
        Self {
            operation_id: hex::encode(rand::random::<[u8; 32]>()),
        }
    }
}

/// Install the synchronous document contract and reconcile it with IndexedDB.
///
/// Installation starts closed: a reload cannot become eligible between Wasm
/// taking ownership of the producer contract and the authoritative hold read.
/// The JS adapter removes the root attribute only after it proves hold absence.
pub fn install_reload_contract() {
    tonk_install_account_setup_safety();
}

/// Publish a hold before the account ceremony can create a credential.
///
/// The adapter compares `/api/health` with this document's immutable build id
/// inside the same exclusive lock used by worker claim/reload callbacks. A
/// successor may win the lock first, but then the comparison refuses this
/// attempt before WebAuthn.
pub(crate) async fn begin() -> Result<Lease, CeremonyError> {
    let lease = Lease::new();
    tonk_begin_account_setup(&lease.operation_id, LEASED_REVISION)
        .await
        .map_err(|error| {
            let detail = describe(&error);
            if detail.contains("account setup is already held") {
                CeremonyError::update_safety(
                    "Account setup is already in progress in this browser. Finish it in the other tab before trying again.",
                    false,
                )
            } else {
                CeremonyError::update_safety(
                    "Tonk could not confirm that this page and its service worker are the same version. Reload Tonk before creating a passkey.",
                    true,
                )
            }
        })?;
    Ok(lease)
}

/// Remove this attempt's exact hold after the worker reports success.
pub(crate) async fn settle(lease: &Lease) -> Result<(), CeremonyError> {
    let removed = tonk_clear_account_setup(&lease.operation_id, LEASED_REVISION)
        .await
        .map_err(|_| retained())?
        .as_bool()
        .ok_or_else(retained)?;
    if removed { Ok(()) } else { Err(retained()) }
}

/// Remove the hold after a ceremony error that proves no credential existed.
pub(crate) async fn abandon_before_credential(lease: &Lease) -> Result<(), CeremonyError> {
    settle(lease).await
}

/// An attempt that crossed credential creation remains update-critical until a
/// later recovery path can prove its durable result. Retrying creation would
/// risk minting a second passkey, so the UI must not offer that action.
pub(crate) fn retained() -> CeremonyError {
    CeremonyError::update_safety(
        "Your passkey may have been created, but Tonk could not prove that account recovery finished. Keep this tab open and do not create another passkey. Reload only after account settings show this device, or contact Tonk support.",
        false,
    )
}

fn describe(error: &JsValue) -> String {
    js_sys::Reflect::get(error, &"message".into())
        .ok()
        .and_then(|value| value.as_string())
        .unwrap_or_else(|| format!("{error:?}"))
}

#[wasm_bindgen::prelude::wasm_bindgen(inline_js = r#"
const NAME = "tonk-update-safety-v1";
const STORE = "holds";
const KEY = "account-setup";
const ATTRIBUTE = "data-tonk-account-setup-critical";
const EVENT = "tonk:account-setup-critical-change";
const MAX_U64 = 18446744073709551615n;

function validate(value) {
    if (value === undefined) return undefined;
    if (value === null || typeof value !== "object" || Array.isArray(value)) {
        throw new Error("malformed account setup hold");
    }
    const keys = Reflect.ownKeys(value).sort();
    const expected = ["kind", "leasedRevision", "operationId", "version"];
    if (keys.length !== expected.length ||
        keys.some((key, index) => typeof key !== "string" || key !== expected[index]) ||
        value.version !== 1 || value.kind !== KEY ||
        typeof value.operationId !== "string" || !/^[0-9a-f]{64}$/.test(value.operationId) ||
        typeof value.leasedRevision !== "string" || !/^(0|[1-9][0-9]*)$/.test(value.leasedRevision) ||
        BigInt(value.leasedRevision) > MAX_U64) {
        throw new Error("malformed account setup hold");
    }
    return value;
}

function setCritical(critical) {
    const root = document.documentElement;
    if (!root) return;
    const previous = root.hasAttribute(ATTRIBUTE);
    if (critical) root.setAttribute(ATTRIBUTE, "");
    else root.removeAttribute(ATTRIBUTE);
    if (previous === critical) return;
    window.dispatchEvent(new CustomEvent(EVENT, {
        detail: { critical, mayReload: !critical },
    }));
}

function announce() {
    const message = { type: "account-setup-hold-changed", version: 1 };
    try {
        const channel = new BroadcastChannel(NAME);
        channel.postMessage(message);
        channel.close();
    } catch {}
    try {
        // Wake the exact incumbent that served this ceremony. It re-reads the
        // durable hold before retiring; the broadcast remains the peer-page
        // wakeup and the worker fetch path remains a missed-message fallback.
        navigator.serviceWorker?.controller?.postMessage(message);
    } catch {}
}

function openDatabase() {
    return new Promise((resolve, reject) => {
        let settled = false;
        const fail = error => {
            if (settled) return;
            settled = true;
            reject(error);
        };
        if (!globalThis.indexedDB) return fail(new Error("IndexedDB unavailable"));
        let request;
        try {
            request = indexedDB.open(NAME, 1);
        } catch (error) {
            return fail(error);
        }
        request.onupgradeneeded = event => {
            try {
                if (event.oldVersion !== 0 || request.result.objectStoreNames.contains(STORE)) {
                    request.transaction?.abort();
                    return;
                }
                request.result.createObjectStore(STORE);
            } catch {
                try { request.transaction?.abort(); } catch {}
            }
        };
        request.onerror = () => fail(request.error || new Error("update safety database open failed"));
        request.onblocked = () => fail(new Error("update safety database open blocked"));
        request.onsuccess = () => {
            const database = request.result;
            if (settled) {
                database.close();
                return;
            }
            settled = true;
            database.onversionchange = () => database.close();
            if (database.version !== 1 || !database.objectStoreNames.contains(STORE)) {
                database.close();
                reject(new Error("unsupported update safety database"));
                return;
            }
            resolve(database);
        };
    });
}

function read(database) {
    return new Promise((resolve, reject) => {
        let value;
        try {
            const transaction = database.transaction(STORE, "readonly");
            const request = transaction.objectStore(STORE).get(KEY);
            request.onsuccess = () => { value = request.result; };
            request.onerror = () => reject(request.error || new Error("account setup hold read failed"));
            transaction.oncomplete = () => resolve(value);
            transaction.onerror = () => reject(transaction.error || new Error("account setup hold transaction failed"));
            transaction.onabort = () => reject(transaction.error || new Error("account setup hold transaction aborted"));
        } catch (error) {
            reject(error);
        }
    });
}

async function durableCritical() {
    const database = await openDatabase();
    try {
        return validate(await read(database)) !== undefined;
    } finally {
        database.close();
    }
}

function putAbsent(database, value) {
    return new Promise((resolve, reject) => {
        try {
            const transaction = database.transaction(STORE, "readwrite");
            const store = transaction.objectStore(STORE);
            const request = store.get(KEY);
            request.onsuccess = () => {
                try {
                    if (validate(request.result) !== undefined) {
                        transaction.abort();
                        return;
                    }
                    const write = store.put(value, KEY);
                    write.onerror = () => { try { transaction.abort(); } catch {} };
                } catch {
                    try { transaction.abort(); } catch {}
                }
            };
            request.onerror = () => reject(request.error || new Error("account setup hold compare failed"));
            transaction.oncomplete = resolve;
            transaction.onerror = () => reject(transaction.error || new Error("account setup hold write failed"));
            transaction.onabort = () => reject(new Error("account setup is already held"));
        } catch (error) {
            reject(error);
        }
    });
}

function clearExact(database, expected) {
    return new Promise((resolve, reject) => {
        let removed = false;
        try {
            const transaction = database.transaction(STORE, "readwrite");
            const store = transaction.objectStore(STORE);
            const request = store.get(KEY);
            request.onsuccess = () => {
                try {
                    const current = validate(request.result);
                    if (current && current.operationId === expected.operationId &&
                        current.leasedRevision === expected.leasedRevision) {
                        const deletion = store.delete(KEY);
                        deletion.onsuccess = () => { removed = true; };
                        deletion.onerror = () => { try { transaction.abort(); } catch {} };
                    }
                } catch {
                    try { transaction.abort(); } catch {}
                }
            };
            request.onerror = () => reject(request.error || new Error("account setup hold compare failed"));
            transaction.oncomplete = () => resolve(removed);
            transaction.onerror = () => reject(transaction.error || new Error("account setup hold clear failed"));
            transaction.onabort = () => reject(transaction.error || new Error("account setup hold clear aborted"));
        } catch (error) {
            reject(error);
        }
    });
}

function withLock(callback) {
    const locks = navigator.locks;
    if (!locks || typeof locks.request !== "function") {
        return Promise.reject(new Error("Web Locks unavailable"));
    }
    return locks.request(NAME, { mode: "exclusive" }, callback);
}

async function assertCompatibleWorker() {
    const page = document.querySelector('meta[name="tonk-worker-build"]')?.content;
    if (!page) throw new Error("page build is absent");
    const healthResponse = await fetch("/api/health", {
        cache: "no-store",
        headers: { "x-tonk-build": page },
    });
    if (!healthResponse.ok) {
        throw new Error(`worker health returned ${healthResponse.status}`);
    }
    const health = await healthResponse.json();
    if (!health || health.build !== page) {
        throw new Error(`stale worker: page ${page}, worker ${health?.build || "unknown"}`);
    }

    // The account provider deploys independently of the immutable page/worker
    // generation. Ask this exact worker to negotiate its provider before the
    // durable hold is published and before credentials.create can run. An old
    // worker, missing route, timeout, malformed marker, or drifted provider all
    // fail on the repeatable side of WebAuthn.
    const capabilityResponse = await fetch("/api/account/setup-capabilities", {
        cache: "no-store",
        headers: { "x-tonk-build": page },
    });
    if (!capabilityResponse.ok) {
        throw new Error(`account setup capability returned ${capabilityResponse.status}`);
    }
    const marker = await capabilityResponse.json();
    const markerKeys = marker && typeof marker === "object" && !Array.isArray(marker)
        ? Reflect.ownKeys(marker).sort()
        : [];
    const capabilities = marker?.capabilities;
    const capabilityKeys = capabilities && typeof capabilities === "object" &&
        !Array.isArray(capabilities)
        ? Reflect.ownKeys(capabilities).sort()
        : [];
    if (
        markerKeys.length !== 2 || markerKeys[0] !== "capabilities" ||
        markerKeys[1] !== "service" || marker.service !== "tonk-access-service" ||
        capabilityKeys.length !== 1 ||
        capabilityKeys[0] !== "accountSetupLifecycle" ||
        capabilities.accountSetupLifecycle !== 1
    ) {
        throw new Error("account setup capability is malformed or unsupported");
    }
}

export function tonk_install_account_setup_safety() {
    setCritical(true);
    if (typeof window.tonkAccountSetupMayReload !== "function") {
        window.tonkAccountSetupMayReload = () =>
            !document.documentElement?.hasAttribute(ATTRIBUTE);
    }
    const reconcile = () => {
        // A wakeup is advisory. Close synchronously, then re-read the durable
        // authority under the shared lock before changing the page predicate.
        setCritical(true);
        withLock(async () => {
            setCritical(await durableCritical());
        }).catch(error => {
            console.warn("account setup safety restore failed; deferring update", error);
        });
    };
    if (!window.__tonkAccountSetupSafetyInstalled) {
        window.__tonkAccountSetupSafetyInstalled = true;
        window.addEventListener("tonk:account-setup-hold-change", reconcile);
    }
    reconcile();
}

export async function tonk_begin_account_setup(operationId, leasedRevision) {
    const value = validate({
        version: 1,
        kind: KEY,
        operationId,
        leasedRevision,
    });
    await withLock(async () => {
        setCritical(true);
        try {
            await assertCompatibleWorker();
            const database = await openDatabase();
            try {
                await putAbsent(database, value);
            } finally {
                database.close();
            }
        } catch (error) {
            // This attempt may have failed because another tab already owns
            // the durable hold. Mirror the authority before reopening the
            // local predicate; if the read itself fails, remain fail-closed.
            try {
                setCritical(await durableCritical());
            } catch {
                setCritical(true);
            }
            throw error;
        }
    });
    announce();
}

export async function tonk_clear_account_setup(operationId, leasedRevision) {
    const expected = validate({
        version: 1,
        kind: KEY,
        operationId,
        leasedRevision,
    });
    const removed = await withLock(async () => {
        const database = await openDatabase();
        try {
            return await clearExact(database, expected);
        } finally {
            database.close();
        }
    });
    if (removed) {
        setCritical(false);
        announce();
    }
    return removed;
}
"#)]
extern "C" {
    fn tonk_install_account_setup_safety();

    #[wasm_bindgen::prelude::wasm_bindgen(catch)]
    async fn tonk_begin_account_setup(
        operation_id: &str,
        leased_revision: &str,
    ) -> Result<(), JsValue>;

    #[wasm_bindgen::prelude::wasm_bindgen(catch)]
    async fn tonk_clear_account_setup(
        operation_id: &str,
        leased_revision: &str,
    ) -> Result<JsValue, JsValue>;
}
