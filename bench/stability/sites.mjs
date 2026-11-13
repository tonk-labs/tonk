// Where each site of the page is, and asking it something.
//
// The app's page at its own origin frames the person's profile at
// `profile.{host}`, and the profile frames the space it shows at
// `{label}.{host}`, the label being the space's key in multibase base32.
// Each site has a worker of its own, which runs its kernel and answers its
// `/api/` routes; the app's origin has only a static worker and answers no
// `/api/` route. So a request about a space goes from the space's frame,
// and one about the profile from the profile's.

const BASE58 = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
const BASE32 = "abcdefghijklmnopqrstuvwxyz234567";

function base58(text) {
  const bytes = [];
  for (const char of text) {
    let carry = BASE58.indexOf(char);
    if (carry < 0) throw new Error(`not base58: ${text}`);
    for (let i = 0; i < bytes.length; i++) {
      carry += bytes[i] * 58;
      bytes[i] = carry & 255;
      carry >>= 8;
    }
    for (; carry; carry >>= 8) bytes.push(carry & 255);
  }
  for (const char of text) {
    if (char !== "1") break;
    bytes.push(0);
  }
  return bytes.reverse();
}

function base32(bytes) {
  let out = "";
  let bits = 0;
  let value = 0;
  for (const byte of bytes) {
    value = (value << 8) | byte;
    bits += 8;
    while (bits >= 5) {
      bits -= 5;
      out += BASE32[(value >> bits) & 31];
    }
  }
  if (bits > 0) out += BASE32[(value << (5 - bits)) & 31];
  return out;
}

/** The label of the space `did`'s origin, as `tonk_host::space_origin::encode_label` writes it. */
export function siteLabel(did) {
  const multibase = did.replace(/^did:key:/, "");
  if (!multibase.startsWith("z")) throw new Error(`not a base58 did:key: ${did}`);
  return `b${base32(base58(multibase.slice(1)))}`;
}

function labelOf(frame) {
  try {
    return new URL(frame.url()).hostname.split(".")[0];
  } catch {
    return "";
  }
}

function depth(frame) {
  let d = 0;
  for (let f = frame; f.parentFrame(); f = f.parentFrame()) d++;
  return d;
}

/** The outermost frame whose origin is the space `did`'s, or null. */
export function spaceFrame(page, did) {
  // A deployment may cut a long label short to fit its suffix beside it;
  // the first forty characters still tell spaces apart.
  const label = siteLabel(did).slice(0, 40);
  const frames = page.frames().filter((f) => labelOf(f).startsWith(label));
  return frames.sort((a, b) => depth(a) - depth(b))[0] ?? null;
}

/**
 * The profile's frame, or null. The app keeps a second frame on the
 * profile's origin (`#connector`) that renders nothing; its worker is the
 * same, so it serves where the rendering one is gone.
 */
export function profileFrame(page) {
  const frames = page.frames().filter((f) => /^profile(-[a-z0-9]+)?$/.test(labelOf(f)));
  return frames.find((f) => !f.url().endsWith("#connector")) ?? frames[0] ?? null;
}

/** `promise`, or `fallback` once `ms` passed: a frame still loading never answers. */
export function within(promise, fallback, ms = 2_000) {
  return Promise.race([promise, new Promise((resolve) => setTimeout(() => resolve(fallback), ms))]);
}

/**
 * Fetch `path` from `frame`, as a page of its origin would, and answer the
 * status and how long it took. A missing frame answers `{ status: "no frame" }`.
 */
export async function timedFetch(frame, path, init = {}) {
  if (!frame) return { status: "no frame", ms: 0 };
  return frame
    .evaluate(
      async ({ path, init }) => {
        const t0 = performance.now();
        const status = await fetch(path, init).then(
          (r) => r.status,
          (e) => String(e),
        );
        return { status, ms: Math.round(performance.now() - t0) };
      },
      { path, init },
    )
    .catch((error) => ({ status: String(error).slice(0, 200), ms: 0 }));
}
