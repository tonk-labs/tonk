import { formatUnits } from 'ethers'

export const formatNumber = (v: string, decimals = 18, visibleDecimals = 5) => {
  try {
    const bv = BigInt(v)

    if (bv === 0n) {
      return '0'
    }

    if (bv < 10n ** BigInt(decimals - 6)) {
      return '< 0.000001'
    }

    const remainder = bv % 10n ** BigInt(decimals - visibleDecimals)

    return formatUnits(bv - remainder, decimals)
  } catch {
    return v
  }
}
