// The `content` envelope — a version-tagged wrapper around the
// editor's markdown, shaped like an email/MIME message: headers, a
// blank line, then the body. Self-describing and cheaply extensible
// (add a header later without breaking older parsers).
//
//   Tonk-Prose-Version: 1
//   ETag: "<hlc>"
//   Content-Type: text/markdown
//
//   <markdown body…>
//
// Following email (RFC 5322) rather than HTTP: there's no leading
// protocol/request line — the format is identified and versioned by a
// header, the way `MIME-Version: 1.0` versions a MIME message. Our
// `Tonk-Prose-Version` header is both the format-version channel and
// the envelope-vs-bare-markdown discriminator: it must be the first
// header line, and real markdown effectively never opens with it, so a
// leading `Tonk-Prose-Version:` reliably marks an envelope (and a
// future `: 2` can be recognized and handled distinctly).
//
// The blank line after the headers separates them from the body; the
// body is raw markdown verbatim and may contain blank lines of its own
// (only the *first* blank line is the separator).
//
// Identity/ordering lives in the `ETag` — HTTP's purpose-built version
// validator, opaque by design. Its value is a Hybrid Logical Clock
// (hlc.ts): a monotonic, cross-node-comparable integer. Why a version
// at all: a plain markdown string carries none, so an incoming write
// that happens to equal a value we sent is indistinguishable from our
// own echo — content-equality can't tell a stale self-echo from a
// remote edit that coincides. The HLC lets the element adopt an
// incoming write only when it is newer (greater HLC) than what it has,
// and ignore anything not newer — which drops our own round-tripped
// echoes without swallowing a genuinely newer remote update.
//
// (A later Automerge swap would put the document's own version
// identity in the ETag, keeping this envelope seam.)

import { parseHlc, formatHlc } from "./hlc";

/** The decoded envelope. `hlc` is null for a bare markdown string
 *  (no version identity → always adopt, treated as newest). */
export interface Content {
  hlc: bigint | null;
  value: string;
}

/** The format-version header — first line of an envelope, à la
 *  `MIME-Version`. Its presence discriminates envelope from bare
 *  markdown; its number versions the envelope format. */
const VERSION_HEADER = "Tonk-Prose-Version";
const VERSION = "1";

/** True when `text` looks like a content envelope (vs bare markdown):
 *  its first line is our version header. Case-insensitive: layers the
 *  envelope passes through (DOM attribute reflection, stores) may
 *  normalize header casing, so `tonk-prose-version:` must still be
 *  recognized — otherwise a mangled envelope is mistaken for bare
 *  markdown and its own headers leak into the document. */
export function isEnvelope(text: string): boolean {
  return text.slice(0, VERSION_HEADER.length + 1).toLowerCase() ===
    `${VERSION_HEADER.toLowerCase()}:`;
}

/** Decode a `content` string into `{ hlc, value }`. Accepts either the
 *  envelope form or a bare markdown string (back-compat): a bare
 *  string decodes to `{ hlc: null, value }` so existing `value=`-style
 *  bindings keep working. A malformed envelope (no blank-line
 *  separator, missing/garbled ETag) degrades to bare markdown rather
 *  than throwing — a parse that aborted here would drop the text. */
export function parseContent(text: string): Content {
  if (!isEnvelope(text)) return { hlc: null, value: text };

  // Split headers from body at the first blank line. Accept CRLF or
  // LF; take the body from the original text to preserve its bytes.
  const sepCrlf = text.indexOf("\r\n\r\n");
  const sepLf = text.indexOf("\n\n");
  let headerEnd: number;
  let bodyStart: number;
  if (sepCrlf !== -1 && (sepLf === -1 || sepCrlf <= sepLf)) {
    headerEnd = sepCrlf;
    bodyStart = sepCrlf + 4;
  } else if (sepLf !== -1) {
    headerEnd = sepLf;
    bodyStart = sepLf + 2;
  } else {
    return { hlc: null, value: text };
  }

  const value = text.slice(bodyStart);
  const lines = text.slice(0, headerEnd).split(/\r\n|\n/);

  let hlc: bigint | null = null;
  for (const line of lines) {
    const colon = line.indexOf(":");
    if (colon === -1) continue;
    const name = line.slice(0, colon).trim().toLowerCase();
    if (name === "etag") {
      hlc = parseHlc(line.slice(colon + 1).trim().replace(/^"|"$/g, ""));
    }
  }

  return { hlc, value };
}

/** Encode a `{ hlc, value }` into the envelope wire form. A null hlc
 *  produces bare markdown (no envelope), matching how it parsed. */
export function formatContent(content: Content): string {
  if (content.hlc === null) return content.value;
  return (
    `${VERSION_HEADER}: ${VERSION}\r\n` +
    `ETag: "${formatHlc(content.hlc)}"\r\n` +
    `Content-Type: text/markdown\r\n` +
    `\r\n` +
    content.value
  );
}
