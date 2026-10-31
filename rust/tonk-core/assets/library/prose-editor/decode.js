// The `content` envelope, shaped like a MIME message: headers, a blank
// line, then the markdown body verbatim.
//
//   Tonk-Prose-Version: 1
//   ETag: "<hlc>"
//   Content-Type: text/markdown
//
// Text not opening with the version header (in any case) is bare
// markdown with no version. A malformed envelope degrades to bare
// markdown rather than dropping the text.
(self, text) => {
  const header = 'tonk-prose-version:';
  if (text.slice(0, header.length).toLowerCase() !== header) return { hlc: null, value: text };
  const crlf = text.indexOf('\r\n\r\n');
  const lf = text.indexOf('\n\n');
  let end;
  let start;
  if (crlf !== -1 && (lf === -1 || crlf <= lf)) {
    end = crlf;
    start = crlf + 4;
  } else if (lf !== -1) {
    end = lf;
    start = lf + 2;
  } else {
    return { hlc: null, value: text };
  }
  let hlc = null;
  for (const line of text.slice(0, end).split(/\r\n|\n/)) {
    const colon = line.indexOf(':');
    if (colon === -1 || line.slice(0, colon).trim().toLowerCase() !== 'etag') continue;
    const digits = line.slice(colon + 1).trim().replace(/^"|"$/g, '');
    hlc = /^\d+$/.test(digits) ? BigInt(digits) : null;
  }
  return { hlc, value: text.slice(start) };
}
