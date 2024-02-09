export const amountFormat = ({amountCents, currency}: {amountCents?: bigint, currency: string}) => {
  return typeof amountCents === 'bigint'
    ? new Intl.NumberFormat(navigator.language, { style: 'currency', currency: currency }).format(Number(amountCents!) / 100)
    : ''
}
