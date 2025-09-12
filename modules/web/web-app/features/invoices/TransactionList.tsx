import { RefreshCwIcon } from 'lucide-react'

import { PaymentMethodDisplay } from '@/features/invoices/PaymentMethodDisplay'
import { TransactionStatusBadge } from '@/features/invoices/TransactionStatusBadge'
import { amountFormat } from '@/features/invoices/amountFormat'
import { Transaction, Transaction_PaymentTypeEnum } from '@/rpc/api/invoices/v1/models_pb'
import { parseAndFormatDate } from '@/utils/date'

interface TransactionListProps {
  transactions: Transaction[]
  currency: string
  isLoading?: boolean
}

export const TransactionList = ({ transactions, currency, isLoading }: TransactionListProps) => {
  if (isLoading) {
    return (
      <div className="flex items-center justify-center py-4">
        <RefreshCwIcon className="animate-spin mr-2" size={16} />
        <span className="text-sm text-muted-foreground">Loading transactions...</span>
      </div>
    )
  }

  if (!transactions?.length) {
    return (
      <div className="text-sm text-muted-foreground py-2">
        No payment transactions
      </div>
    )
  }

  return (
    <div className="space-y-2">
      {transactions.map(transaction => (
        <div key={transaction.id} className="py-2">
          <div className="flex justify-between items-start">
            <div className="flex-1">
              <div className="flex items-center gap-2">
                <PaymentMethodDisplay paymentMethodInfo={transaction.paymentMethodInfo} compact />
                <TransactionStatusBadge status={transaction.status} />
              </div>
              <div className="text-[11px] text-muted-foreground mt-1 ml-5">
                {transaction.processedAt ? parseAndFormatDate(transaction.processedAt) : 'Pending processing'}
              </div>
            </div>
            <div className="text-right">
              <div className="text-[13px] font-mono">
                {transaction.paymentType === Transaction_PaymentTypeEnum.REFUND && '-'}
                {amountFormat({
                  currency,
                  total: transaction.amount,
                })}
              </div>
            </div>
          </div>
        </div>
      ))}
    </div>
  )
}

export default TransactionList
