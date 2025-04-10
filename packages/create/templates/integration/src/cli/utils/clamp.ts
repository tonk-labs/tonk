/**
 * Clamps the value to the range [min, max] and loops if looped is true.
 */
export const clamp = (n: number, min: number, max: number, looped = false) => {
  if (looped) {
    if (n < 0) {
      return max;
    }

    if (n > max) {
      return min;
    }
  }

  return Math.max(min, Math.min(max, n));
};
