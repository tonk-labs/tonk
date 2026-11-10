// Spaces an earlier release of Tonk kept in this page's origin.
//
// That release ran one worker here, on the app's origin, and kept every
// profile and space in this origin's storage. This release keeps the profile
// and each space on an origin of its own, which cannot read this one's. So
// this page offers, once, to bring over the spaces that release kept on this
// device alone: it reads them with a bundle it loads only then, and hands
// each to the profile's worker, which has the persona make a space as a copy
// of it. A space that syncs comes back with its account when the person signs
// in, so nothing of it is read. What the earlier release kept is left where it
// is.

// The databases only the earlier release made in this origin: the store of
// the keys every profile of it signed with, and the registry profile of the
// releases before profiles were kept by their DID. This release keeps nothing
// in this origin's databases.
export const LEFT_BEHIND = ["dialog.credential", "tonk"];
// Where this browser remembers what came of the offer.
export const ANSWER = "tonk-migration";
// How long the profile's worker has to make the spaces handed to it.
const HAND_OVER_MS = 120_000;
// How long this page waits for the profile's frame to be there to hand to.
const FRAME_WAIT_MS = 60_000;

// Whether `databases`, as `indexedDB.databases()` lists them, hold what the
// earlier release kept.
export function leftBehind(databases) {
    return (databases ?? []).some(database => LEFT_BEHIND.includes(database?.name));
}

// What this browser answered the offer, if it did. Storage that cannot be
// read reads as no answer.
export function answered(storage) {
    try {
        return storage?.getItem(ANSWER) ?? null;
    } catch {
        return null;
    }
}

export function remember(storage, answer) {
    try {
        storage?.setItem(ANSWER, answer);
    } catch {
        // A browser that keeps nothing asks again next time.
    }
}

// Whether to offer: the earlier release left something, and this browser has
// not answered.
export async function shouldOffer({ indexedDB, storage }) {
    if (answered(storage)) return false;
    if (typeof indexedDB?.databases !== "function") return false;
    try {
        return leftBehind(await indexedDB.databases());
    } catch {
        return false;
    }
}

// Hand `spaces` to the profile's worker through `relay`, the page's way to
// it (`tonkProfileWorker`): answers `{ made, failed }` as it does.
export function handOverThrough(relay, { timeout = HAND_OVER_MS, MessageChannel = globalThis.MessageChannel } = {}) {
    return spaces =>
        new Promise((resolve, reject) => {
            const channel = new MessageChannel();
            const timer = setTimeout(() => {
                channel.port1.close();
                reject(new Error("the profile did not answer"));
            }, timeout);
            channel.port1.onmessage = ({ data }) => {
                clearTimeout(timer);
                channel.port1.close();
                resolve(data);
            };
            const handed = spaces.map(({ name, content, revision }) => ({
                name,
                content: content.slice().buffer,
                revision: revision.slice().buffer,
            }));
            const buffers = handed.flatMap(({ content, revision }) => [content, revision]);
            relay({ type: "migrate", spaces: handed }, [channel.port2, ...buffers]);
        });
}

// Read what the earlier release kept on this device alone, with `load`, and
// hand it over with `handOver`.
export async function bringOver({ load, handOver, log = () => {} }) {
    const spaces = await load();
    log(`read ${spaces.length} space(s) kept on this device alone`);
    if (spaces.length === 0) return { made: [], failed: [] };
    const outcome = await handOver(spaces);
    log(`made ${outcome.made.length}, failed ${outcome.failed.length}`);
    return outcome;
}

// The bundle that reads what the earlier release kept, loaded only now.
async function loadKept() {
    const bundle = await import("/migrate/migrate.js");
    await bundle.default();
    return Array.from(await bundle.localSpaces());
}

// What to say of `outcome`, `{ made, failed }`.
export function said({ made, failed }) {
    if (failed.length > 0) {
        const names = failed.map(({ name }) => name).join(", ");
        return `Couldn't bring over ${names}. They are still here, and you can try again.`;
    }
    if (made.length === 0) {
        return "No space was kept only on this device. Spaces that sync come back when you sign in.";
    }
    return made.length === 1
        ? "Brought over 1 space. You'll find it in your hub."
        : `Brought over ${made.length} spaces. You'll find them in your hub.`;
}

const STYLE = `
.tonk-migration {
    position: fixed; inset: auto 16px 16px auto; z-index: 2147483000;
    max-width: min(420px, calc(100vw - 32px)); box-sizing: border-box;
    padding: 16px; border-radius: 12px; font: 400 15px/1.4 system-ui, sans-serif;
    background: #fff; color: #1d1d1f; box-shadow: 0 8px 32px rgb(0 0 0 / 0.18);
}
.tonk-migration h2 { margin: 0 0 8px; font-size: 16px; font-weight: 600; }
.tonk-migration p { margin: 0 0 12px; }
.tonk-migration [data-actions] { display: flex; gap: 8px; justify-content: flex-end; }
.tonk-migration button { font: inherit; padding: 6px 14px; border-radius: 8px; border: 1px solid #c7c7cc; background: #fff; color: inherit; cursor: pointer; }
.tonk-migration button[data-bring] { background: #1d1d1f; border-color: #1d1d1f; color: #fff; }
@media (prefers-color-scheme: dark) {
    .tonk-migration { background: #1c1c1e; color: #f2f2f7; }
    .tonk-migration button { background: #2c2c2e; border-color: #48484a; }
    .tonk-migration button[data-bring] { background: #f2f2f7; border-color: #f2f2f7; color: #1c1c1e; }
}`;

// Show the offer in `document`. `bring` is run on accepting, and answers the
// outcome to say.
export function offer(document, { bring, decline }) {
    const style = document.createElement("style");
    style.textContent = STYLE;
    const card = document.createElement("section");
    card.className = "tonk-migration";
    card.setAttribute("role", "dialog");
    card.setAttribute("aria-labelledby", "tonk-migration-title");
    card.innerHTML = `
        <h2 id="tonk-migration-title">Bring over spaces from the previous version</h2>
        <p data-status role="status">Spaces you kept only on this device in the previous version of Tonk are still here. Bring them over to keep working on them. The originals stay where they are.</p>
        <div data-actions>
            <button type="button" data-later>Not now</button>
            <button type="button" data-bring>Bring over</button>
        </div>`;
    const status = card.querySelector("[data-status]");
    const actions = card.querySelector("[data-actions]");
    const close = () => {
        card.remove();
        style.remove();
    };
    card.querySelector("[data-later]").addEventListener("click", () => {
        decline();
        close();
    });
    card.querySelector("[data-bring]").addEventListener("click", async () => {
        actions.hidden = true;
        status.textContent = "Bringing over your spaces…";
        const words = await bring();
        status.textContent = words;
        actions.innerHTML = '<button type="button" data-done>Done</button>';
        actions.hidden = false;
        actions.querySelector("[data-done]").addEventListener("click", close);
    });
    document.head.append(style);
    document.body.append(card);
    return card;
}

// The page's way to the profile's worker, once the profile's frame is there.
async function profileRelay(window) {
    const started = Date.now();
    while (typeof window.tonkProfileWorker !== "function") {
        if (Date.now() - started > FRAME_WAIT_MS) return null;
        await new Promise(resolve => setTimeout(resolve, 250));
    }
    return window.tonkProfileWorker;
}

// Offer to bring over what the earlier release kept, if it left anything and
// this browser has not answered.
export async function install(window) {
    let storage = null;
    try {
        storage = window.localStorage;
    } catch {
        // Offered every time where nothing can be remembered.
    }
    if (!(await shouldOffer({ indexedDB: window.indexedDB, storage }))) return;
    const relay = await profileRelay(window);
    if (!relay) return;
    offer(window.document, {
        decline: () => remember(storage, "declined"),
        bring: async () => {
            try {
                const outcome = await bringOver({
                    load: loadKept,
                    handOver: handOverThrough(relay),
                    log: (...said) => console.log("[migration]", ...said),
                });
                if (outcome.failed.length === 0) remember(storage, "brought");
                return said(outcome);
            } catch (error) {
                console.warn("[migration]", error);
                return "Couldn't read the spaces the previous version kept. They are still here, and you can try again.";
            }
        },
    });
}
