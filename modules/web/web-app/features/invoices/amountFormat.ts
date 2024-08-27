import { Invoice } from '@/rpc/api/invoices/v1/models_pb'

export const amountFormat = ({ total, currency }: Invoice) => {
  return typeof total === 'bigint'
    ? new Intl.NumberFormat(navigator.language, { style: 'currency', currency: currency }).format(
        Number(total!) / 100
      )
    : ''
}
