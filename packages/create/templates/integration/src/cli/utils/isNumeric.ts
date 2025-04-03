export const isNumeric = (n: string) => {
  return !isNaN(parseFloat(n)) && isFinite(n as any)
}
