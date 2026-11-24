// Merge a version seen on an incoming write into the clock, so the next
// local edit is ordered after it.
(self, remote) => {
  const state = self.__prose;
  const mine = state.clock >> 16n;
  const theirs = remote >> 16n;
  const now = BigInt(Date.now());
  const physical = [mine, theirs, now].reduce((a, b) => (a > b ? a : b));
  let logical = 0n;
  if (physical === mine && physical === theirs) {
    const a = state.clock & 0xffffn;
    const b = remote & 0xffffn;
    logical = (a > b ? a : b) + 1n;
  } else if (physical === mine) {
    logical = (state.clock & 0xffffn) + 1n;
  } else if (physical === theirs) {
    logical = (remote & 0xffffn) + 1n;
  }
  state.clock = (physical << 16n) | (logical & 0xffffn);
  return state.clock;
}
