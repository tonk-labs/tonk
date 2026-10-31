// A Hybrid Logical Clock: wall-clock millis in the high 48 bits, a
// counter in the low 16, so versions order by integer compare and never
// run backwards with the system clock.
(self) => {
  const state = self.__prose;
  const last = state.clock >> 16n;
  const now = BigInt(Date.now());
  const physical = now > last ? now : last;
  const logical = physical === last ? (state.clock & 0xffffn) + 1n : 0n;
  state.clock = (physical << 16n) | (logical & 0xffffn);
  return state.clock;
}
