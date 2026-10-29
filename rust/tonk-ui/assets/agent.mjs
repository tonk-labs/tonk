// The OAuth approval page, `/settings/agent?request=<handle>`.
//
// An LLM host sends the user to `/authorize`; the authorization server
// records the request and redirects here. The page says who is asking
// and where the answer goes, lets the user name the agent and choose
// the spaces it may reach, then asks the service worker for a new agent
// and its session token (`POST /api/agent/session`) and hands the token
// to the authorization server (`POST /authorize/<handle>`), which answers
// with the callback to send the browser to.
//
// Like `/doctor`, this route mounts without the shell's Wasm. It needs
// the service worker an earlier visit to Tonk installed, and a signed-in
// account whose spaces sync.

export const ACCESS = [
    ["none", "No access"],
    ["read", "Read"],
    ["write", "Read and write"],
];

// Hosts on this machine: a callback there is only as trustworthy as
// whatever is listening on the port.
export function isLoopback(host) {
    return host === "localhost" || host === "[::1]" || host === "::1" || /^127\./.test(host ?? "");
}

// The request body for `/api/agent/session`: the spaces with some access.
export function sessionRequest(name, choices) {
    return {
        name: name.trim(),
        spaces: choices
            .filter(choice => choice.access === "read" || choice.access === "write")
            .map(({ key, access }) => ({ key, access })),
    };
}

// The message of a worker or authorization-server error body.
export function errorMessage(body, status) {
    if (typeof body?.error === "object" && body.error?.message) return body.error.message;
    if (typeof body?.error_description === "string") return body.error_description;
    if (typeof body?.error === "string") return body.error;
    return `Request failed (HTTP ${status})`;
}

async function call(path, init) {
    const response = await fetch(path, { credentials: "same-origin", ...init });
    const text = await response.text();
    let body;
    try { body = text ? JSON.parse(text) : null; } catch { body = null; }
    if (!response.ok) throw new Error(errorMessage(body, response.status));
    return body;
}

const post = (path, body) => call(path, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
});

const styles = `
#tonk-agent { max-width: 720px; margin: 0 auto; padding: 36px 24px 72px;
    font-family: "IBM Plex Sans", system-ui, sans-serif; line-height: 1.5;
    color: var(--wa-color-text-normal, light-dark(#38182a, #e2dfdd));
    -webkit-font-smoothing: antialiased; }
#tonk-agent *, #tonk-agent *::before, #tonk-agent *::after { box-sizing: border-box; }
#tonk-agent h1, #tonk-agent h2 { display: block; background: none; color: inherit; padding: 0;
    font-family: inherit; letter-spacing: -.025em; line-height: 1.2; text-wrap: balance; }
#tonk-agent h1 { font-size: clamp(1.75rem, 5vw, 2.5rem); margin: 20px 0 12px; }
#tonk-agent h2 { font-size: 1.05rem; margin: 0 0 12px; }
#tonk-agent p { text-wrap: pretty; margin: 0 0 16px; max-width: 65ch; }
#tonk-agent section { margin: 0 0 16px; padding: 24px;
    background: color-mix(in srgb, var(--wa-color-surface-default, light-dark(#fff, #161313)) 65%, transparent);
    box-shadow: 0 0 0 1px color-mix(in srgb, currentColor 12%, transparent); }
#tonk-agent label { display: block; font-size: .875rem; font-weight: 600; }
#tonk-agent input, #tonk-agent select { width: 100%; min-height: 44px; margin: 6px 0 0; padding: 8px 12px;
    font: inherit; color: inherit; background: transparent; border-radius: 0;
    border: 1px solid color-mix(in srgb, currentColor 30%, transparent); }
#tonk-agent select option { color: initial; }
#tonk-agent .space { display: grid; grid-template-columns: 1fr minmax(10rem, 14rem); gap: 12px;
    align-items: center; padding: 10px 0;
    border-top: 1px solid color-mix(in srgb, currentColor 10%, transparent); }
#tonk-agent .space:first-of-type { border-top: 0; }
#tonk-agent .space label { font-weight: 400; font-size: 1rem; }
#tonk-agent .space select { margin: 0; }
#tonk-agent .space .why { grid-column: 1 / -1; margin: -4px 0 0; font-size: .8rem; opacity: .7; }
#tonk-agent .tools { display: flex; flex-wrap: wrap; gap: 10px; margin: 20px 0 0; }
#tonk-agent button { min-height: 44px; padding: 9px 18px; font: inherit; color: inherit; border-radius: 0;
    background: transparent; border: 1px solid color-mix(in srgb, currentColor 35%, transparent); cursor: pointer; }
#tonk-agent button.primary { background: var(--wa-color-text-normal, light-dark(#38182a, #e2dfdd));
    color: var(--wa-color-surface-default, light-dark(#fff, #161313)); }
#tonk-agent button:disabled { opacity: .5; cursor: wait; }
#tonk-agent :focus-visible { outline: 2px solid currentColor; outline-offset: 4px; }
#tonk-agent .meta { font-size: .85rem; opacity: .75; overflow-wrap: anywhere; }
#tonk-agent .warning { border-left: 3px solid currentColor; padding-left: 12px; }
#tonk-agent [role=status], #tonk-agent [role=alert] { margin: 12px 0 0; font-size: .9rem; }
#tonk-agent [role=status]:empty, #tonk-agent [role=alert]:empty { margin: 0; }
@media (max-width: 520px) { #tonk-agent .space { grid-template-columns: 1fr; gap: 4px; } }
`;

function element(tag, properties = {}, children = []) {
    const node = Object.assign(document.createElement(tag), properties);
    for (const child of children) node.append(child);
    return node;
}

// Wait for the service worker to control this page: a navigation under
// its scope normally is, but a hard reload is not. Reload once if a
// registration is active; otherwise Tonk has never run in this browser.
async function controlled() {
    if (navigator.serviceWorker?.controller) return true;
    const registration = await navigator.serviceWorker?.getRegistration?.();
    if (registration?.active && !sessionStorage.getItem("tonk:agent-reload")) {
        sessionStorage.setItem("tonk:agent-reload", "1");
        location.reload();
        return new Promise(() => {});
    }
    return false;
}

async function grantableSpaces() {
    const profile = await call("/api/profile");
    const spaces = [];
    for (const { key, subject } of profile?.space ?? []) {
        let label = key;
        let synced = false;
        try {
            const info = await call(`/api/repository/${encodeURIComponent(key)}`);
            label = info?.label || key;
            synced = Object.keys(info?.remote ?? {}).length > 0;
        } catch { /* listed, but not offered */ }
        spaces.push({ key, subject, label, synced });
    }
    return spaces.sort((a, b) => a.label.localeCompare(b.label));
}

export async function mountAgent() {
    document.title = "Connect an agent · Tonk";
    document.querySelector("#tonk-boot")?.remove();
    document.head.append(element("style", { textContent: styles }));
    const main = element("main", { id: "tonk-agent" });
    document.body.append(main);
    const status = element("p", { role: "status" });
    status.dataset.testid = "agent-status";
    const alert = element("p", { role: "alert" });
    const fail = message => { status.textContent = ""; alert.textContent = message; };

    main.append(element("h1", { textContent: "Connect an agent" }), status, alert);

    const handle = new URLSearchParams(location.search).get("request");
    if (!handle) return fail("This page is opened by an app asking to connect to Tonk. No request was given.");

    let request;
    try {
        request = await call(`/authorize/${encodeURIComponent(handle)}`);
    } catch (error) {
        return fail(`This request can't be answered: ${error.message}. Start again from the app.`);
    }
    const client = request.client_name || "An app";
    const host = request.redirect_host || request.redirect_uri;

    const who = element("section", {}, [
        element("p", {}, [element("strong", { textContent: client }),
            " wants its own agent in your Tonk, working in the spaces you choose below."]),
        element("p", { className: "meta" }, [`After you answer, you go back to ${host}.`]),
    ]);
    if (isLoopback(request.redirect_host)) {
        who.append(element("p", { className: "warning", textContent:
            `${host} is this computer. Approve only if you just started ${client} here yourself.` }));
    }
    main.insertBefore(who, status);

    const deny = element("button", { type: "button", textContent: "Deny" });
    const answer = async body => {
        const { redirect } = await post(`/authorize/${encodeURIComponent(handle)}`, body);
        location.assign(redirect);
    };
    const tools = element("div", { className: "tools" }, [deny]);
    main.insertBefore(tools, status);
    deny.addEventListener("click", async () => {
        deny.disabled = true;
        status.textContent = "Telling the app you said no…";
        try { await answer({ deny: true }); } catch (error) { deny.disabled = false; fail(error.message); }
    });

    status.textContent = "Loading your spaces…";
    if (!(await controlled())) {
        return fail("Tonk isn't running in this browser yet. Open Tonk here, sign in, then come back to this page.");
    }
    let account;
    let spaces;
    try {
        account = await call("/api/account");
        spaces = await grantableSpaces();
    } catch (error) {
        return fail(`Couldn't read your spaces: ${error.message}`);
    }
    if (!account?.provider) {
        return fail("Sign in to your Tonk account first, then come back to this page.");
    }
    status.textContent = "";

    const name = element("input", { id: "agent-name", type: "text", value: client, maxLength: 80,
        required: true, autocomplete: "off" });
    const naming = element("section", {}, [
        element("label", { htmlFor: "agent-name", textContent: "Agent name" }), name,
        element("p", { className: "meta", textContent:
            "The agent gets a space of its own with this name. It can always read and write there." }),
    ]);
    const choices = [];
    const list = element("section", {}, [element("h2", { textContent: "What it can reach" })]);
    if (!spaces.length) list.append(element("p", { className: "meta", textContent: "You have no spaces yet." }));
    for (const space of spaces) {
        const id = `space-${space.key}`;
        const select = element("select", { id, disabled: !space.synced },
            ACCESS.map(([value, text]) => element("option", { value, textContent: text })));
        select.dataset.testid = `space-access-${space.key}`;
        const row = element("div", { className: "space" }, [
            element("label", { htmlFor: id, textContent: space.label }), select]);
        if (!space.synced) {
            row.append(element("p", { className: "why", textContent:
                "Only on this device. Sync it to let a hosted agent reach it." }));
        }
        list.append(row);
        choices.push({ key: space.key, select });
    }
    list.append(element("p", { className: "meta", textContent:
        "Access lasts 90 days. Read lets the agent see a space; read and write lets it change it too." }));

    const approve = element("button", { type: "button", className: "primary", textContent: "Approve" });
    tools.prepend(approve);
    main.insertBefore(naming, tools);
    main.insertBefore(list, tools);

    approve.addEventListener("click", async () => {
        if (!name.value.trim()) { name.focus(); return fail("Give the agent a name."); }
        approve.disabled = deny.disabled = true;
        alert.textContent = "";
        status.textContent = "Creating the agent and its access…";
        try {
            const { token } = await post("/api/agent/session", sessionRequest(name.value,
                choices.map(({ key, select }) => ({ key, access: select.value }))));
            status.textContent = `Sending you back to ${host}…`;
            await answer({ token });
        } catch (error) {
            approve.disabled = deny.disabled = false;
            fail(error.message);
        }
    });
}
