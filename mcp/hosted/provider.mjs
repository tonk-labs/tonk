// Provider routing is deployment configuration, never a tool argument.
export function providerOrigin(value) {
  const url = new URL(value);
  if (url.protocol !== 'https:' || url.origin !== value || url.username || url.password) {
    throw new Error('Expected an HTTPS Tonk provider origin.');
  }
  return url.origin;
}
