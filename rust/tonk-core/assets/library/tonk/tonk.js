// Tonk's data API for a page of a space: query, subscribe, transact and
// evaluate, each a request to the routes the space's own worker serves.
//
//   import { query, subscribe, transact, evaluate } from "/tonk.js";
//
// Importing it also puts the four on `window.tonk`, where a page has one
// and it does not carry them already.
//
// Every call takes the branch to ask last, as `"branch@repository"` or
// `{ with: "branch@repository" }`, and asks the page's own when it is left
// out.

const context = () => globalThis.tonk?.context ?? {};

// The repository and branch a call addresses.
const address = at => {
  const named = (typeof at === "string" ? at : at?.with) || context().with || "";
  const split = named.indexOf("@");
  const branch = split < 0 ? "" : named.slice(0, split);
  const repository = split < 0 ? named : named.slice(split + 1);
  return {
    repository: repository || context().repo || "profile:tonk",
    branch: branch || context().branch || "main",
  };
};

const endpoint = (operation, at) => {
  const { repository, branch } = address(at);
  return `/api/repository/${repository}/branch/${branch}/${operation}`;
};

// What a refused request said, as an error.
const refusal = async response => {
  const text = await response.text();
  let message = text;
  try {
    message = JSON.parse(text).error?.message ?? text;
  } catch {}
  return new Error(message || `HTTP ${response.status}`);
};

const post = async (url, headers, body) => {
  const response = await fetch(url, { method: "POST", headers, body });
  if (!response.ok) throw await refusal(response);
  return response;
};

// A response's JSON, or nothing when it has no body.
const answer = async response => {
  const text = await response.text();
  return text ? JSON.parse(text) : undefined;
};

const JSON_BODY = { "content-type": "application/json" };

// Queries in flight, by what they ask: the same question asked twice at
// once is asked of the worker once.
const asking = new Map();

// The rows `body` matches now.
export const query = (body, at) => {
  const url = endpoint("query", at);
  const text = JSON.stringify(body);
  const key = `${url}\n${text}`;
  let rows = asking.get(key);
  if (!rows) {
    rows = post(url, JSON_BODY, text)
      .then(answer)
      .finally(() => asking.delete(key));
    asking.set(key, rows);
  }
  return rows;
};

// Commit the claims `request` makes.
export const transact = async (request, at) =>
  answer(await post(endpoint("transact", at), JSON_BODY, JSON.stringify(request)));

// Evaluate a notation document: `{ document, transact }`, committed unless
// `transact` is `false`.
export const evaluate = async (detail, at) => {
  const dry = detail?.transact === false ? "?transact=false" : "";
  return answer(
    await post(`${endpoint("evaluate", at)}${dry}`, { "content-type": "text/yaml" }, detail?.document ?? ""),
  );
};

// Which facts of an entity a row speaks for: each field, and for a keyed
// collection (one row per entry) the entry's key too.
const slots = row => {
  const out = new Set();
  for (const [name, value] of Object.entries(row.fields ?? {})) {
    if (name === "this") continue;
    const keys = value && typeof value === "object" && !Array.isArray(value) ? Object.keys(value) : [];
    out.add(`${name}\u001e${keys.length === 1 ? keys[0] : ""}`);
  }
  return out;
};

// The rows after a change. A retracted row leaves by value. Where a
// retraction matches no row kept here, the rows kept for that entity have
// drifted from the worker's, and an asserted row replaces any of them that
// speaks for the same fact, so a superseded value leaves one row and not
// two.
const apply = (rows, { asserted = [], retracted = [] }) => {
  const gone = new Set(retracted.map(row => JSON.stringify(row)));
  const drifted = new Set(retracted.map(row => row.this));
  const claimed = new Map();
  for (const row of asserted) {
    const held = claimed.get(row.this) ?? new Set();
    for (const slot of slots(row)) held.add(slot);
    claimed.set(row.this, held);
  }
  const kept = rows.filter(row => {
    if (!gone.has(JSON.stringify(row))) return true;
    drifted.delete(row.this);
    return false;
  });
  return kept
    .filter(row => {
      const held = drifted.has(row.this) && claimed.get(row.this);
      return !held || ![...slots(row)].some(slot => held.has(slot));
    })
    .concat(asserted);
};

// The rows `body` matches, as a stream: what matches now, then every row
// that matches after each change. Cancelling the stream ends the request.
export const subscribe = (body, at) => {
  const abort = new AbortController();
  return new ReadableStream({
    async start(controller) {
      try {
        const response = await fetch(endpoint("query", at), {
          method: "POST",
          signal: abort.signal,
          headers: { ...JSON_BODY, accept: "text/event-stream" },
          body: JSON.stringify(body),
        });
        if (!response.ok) throw await refusal(response);
        const reader = response.body.pipeThrough(new TextDecoderStream()).getReader();
        let rows = [];
        let text = "";
        for (;;) {
          const { value, done } = await reader.read();
          if (done) break;
          text += value;
          for (let end; (end = text.indexOf("\n\n")) >= 0; text = text.slice(end + 2)) {
            const frame = text.slice(0, end);
            if (!frame.startsWith("data:")) continue;
            const said = JSON.parse(frame.slice(5));
            if (Array.isArray(said)) rows = said;
            else if (said.kind === "snapshot") rows = said.conclusions ?? [];
            else if (said.kind === "delta") rows = apply(rows, said);
            else continue;
            controller.enqueue(rows);
          }
        }
        controller.close();
      } catch (error) {
        if (!abort.signal.aborted) controller.error(error);
      }
    },
    cancel() {
      abort.abort();
    },
  });
};

const tonk = globalThis.tonk;
if (tonk) {
  for (const [name, call] of Object.entries({ query, subscribe, transact, evaluate })) {
    if (!(name in tonk)) tonk[name] = call;
  }
}
