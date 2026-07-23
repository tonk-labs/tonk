// The `content` envelope — a version-tagged wrapper around the
// workbook's serialized bytes, shaped like an email/MIME message:
// headers, a blank line, then the body. The same seam as tonk-prose's
// envelope (see that crate's `content.ts` for the full rationale);
// only the header name, the body encoding, and the media type differ.
//
//   Tonk-Table-Version: 1
//   ETag: "<hlc>"
//   Content-Type: application/vnd.ironcalc
//
//   <base64 of Model.toBytes()…>
//
// `Tonk-Table-Version` is both the format-version channel and the
// envelope-vs-bare-text discriminator: it must be the first header
// line, and real CSV/tabular text never opens with it, so a leading
// `Tonk-Table-Version:` reliably marks an envelope.
//
// Two body encodings are recognized via `Content-Type`:
//
// - `application/vnd.ironcalc` — the body is base64 of the engine's
//   own binary serialization (`Model.toBytes()`), the LOSSLESS channel
//   the element round-trips through a store. Base64 rather than raw
//   bytes because the envelope travels as element text / store text.
// - anything else (or a bare, envelope-less string) — the body is CSV
//   and seeds a fresh workbook: the human-authorable channel, so
//   `<tonk-table>a,b\n1,2</tonk-table>` just works. Formulas survive
//   as `=…` cell text.
//
// Identity/ordering lives in the `ETag`: a Hybrid Logical Clock
// (hlc.ts), monotonic and cross-node comparable. The element adopts an
// incoming write only when its HLC is newer than what it has issued or
// seen, which drops its own round-tripped echoes without swallowing a
// genuinely newer remote update — same protocol as tonk-prose.

import { parseHlc, formatHlc } from "./hlc";

/** The decoded envelope. `hlc` is null for a bare (envelope-less)
 *  string — no version identity → always adopt, treated as newest.
 *  `contentType` is null for a bare string or an envelope that names
 *  no Content-Type; the body is CSV in either case. */
export interface Content {
  hlc: bigint | null;
  contentType: string | null;
  value: string;
}

/** The media type of the lossless body encoding: base64 of the
 *  IronCalc engine's own binary workbook serialization. */
export const WORKBOOK_TYPE = "application/vnd.ironcalc";

/** True when `contentType` names the lossless workbook encoding (the
 *  body is base64 engine bytes, not CSV). Substring match so a future
 *  suffixed form (`application/vnd.ironcalc;v=2`) still routes to the
 *  bytes path. */
export function isWorkbookType(contentType: string | null): boolean {
  return contentType !== null && contentType.toLowerCase().includes("vnd.ironcalc");
}

/** The format-version header — first line of an envelope, à la
 *  `MIME-Version`. Its presence discriminates envelope from bare
 *  text; its number versions the envelope format. */
const VERSION_HEADER = "Tonk-Table-Version";
const VERSION = "1";

/** True when `text` looks like a content envelope (vs bare CSV): its
 *  first line is our version header. Case-insensitive: layers the
 *  envelope passes through (DOM attribute reflection, stores) may
 *  normalize header casing, so `tonk-table-version:` must still be
 *  recognized — otherwise a mangled envelope is mistaken for CSV and
 *  its own headers leak into the grid. */
export function isEnvelope(text: string): boolean {
  return text.slice(0, VERSION_HEADER.length + 1).toLowerCase() ===
    `${VERSION_HEADER.toLowerCase()}:`;
}

/** Decode a `content` string into `{ hlc, contentType, value }`.
 *  Accepts either the envelope form or a bare string: a bare string
 *  decodes to `{ hlc: null, contentType: null, value }` so authored
 *  CSV content and `value=`-style bindings keep working. A malformed
 *  envelope (no blank-line separator) degrades to a bare string
 *  rather than throwing — a parse that aborted here would drop the
 *  content. */
export function parseContent(text: string): Content {
  if (!isEnvelope(text)) return { hlc: null, contentType: null, value: text };

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
    return { hlc: null, contentType: null, value: text };
  }

  const value = text.slice(bodyStart);
  const lines = text.slice(0, headerEnd).split(/\r\n|\n/);

  let hlc: bigint | null = null;
  let contentType: string | null = null;
  for (const line of lines) {
    const colon = line.indexOf(":");
    if (colon === -1) continue;
    const name = line.slice(0, colon).trim().toLowerCase();
    const rest = line.slice(colon + 1).trim();
    if (name === "etag") {
      hlc = parseHlc(rest.replace(/^"|"$/g, ""));
    } else if (name === "content-type") {
      contentType = rest;
    }
  }

  return { hlc, contentType, value };
}

/** Encode a `{ hlc, contentType, value }` into the envelope wire form.
 *  A null hlc produces the bare value (no envelope), matching how it
 *  parsed; a null contentType omits the header (the body reads as
 *  CSV). */
export function formatContent(content: Content): string {
  if (content.hlc === null) return content.value;
  const typeHeader =
    content.contentType === null ? "" : `Content-Type: ${content.contentType}\r\n`;
  return (
    `${VERSION_HEADER}: ${VERSION}\r\n` +
    `ETag: "${formatHlc(content.hlc)}"\r\n` +
    typeHeader +
    `\r\n` +
    content.value
  );
}
