(self) => {
  const value = self.stored();
  if (value.slice(0, 19).toLowerCase() !== 'tonk-prose-version:') return value;
  const crlf = value.indexOf('\r\n\r\n');
  const lf = value.indexOf('\n\n');
  if (crlf !== -1 && (lf === -1 || crlf <= lf)) return value.slice(crlf + 4);
  if (lf !== -1) return value.slice(lf + 2);
  return value;
}
