
export const amountFormat = ({ total, currency }: { total: bigint, currency: string }) => {
  return typeof total === 'bigint'
    ? new Intl.NumberFormat(navigator.language, { style: 'currency', currency: currency }).format(
        Number(total!) / 100
      )
    : ''
}
