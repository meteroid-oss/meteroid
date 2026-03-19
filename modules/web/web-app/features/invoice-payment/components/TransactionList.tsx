import { Ban, CheckCircle, Clock, XCircle } from 'lucide-react'

import { CardBrandLogo } from '@/features/checkout/components/CardBrandLogo'
import { TransactionStatusBadge } from '@/features/invoices/TransactionStatusBadge'
import { Transaction, Transaction_PaymentStatusEnum } from '@/rpc/api/invoices/v1/models_pb'
import { formatCurrency } from '@/utils/numbers'

interface TransactionListProps {
  transactions: Transaction[]
  currency: string
}

export const TransactionList: React.FC<TransactionListProps> = ({ transactions, currency }) => {
  if (transactions.length === 0) {
    return null
  }

  return (
    <div className="">
      <h3 className="text-sm font-medium text-gray-900 mb-4">Transactions</h3>
      <div className="space-y-3">
        {transactions.map(transaction => (
          <TransactionItem key={transaction.id} transaction={transaction} currency={currency} />
        ))}
      </div>
    </div>
  )
}

interface TransactionItemProps {
  transaction: Transaction
  currency: string
}

const TransactionItem: React.FC<TransactionItemProps> = ({ transaction, currency }) => {
  const statusConfig = getStatusConfig(transaction.status)
  const isRefund = transaction.paymentType === 1 // REFUND

  return (
    <div className="flex items-center justify-between p-4 border border-gray-200 rounded-lg">
      <div className="flex items-center flex-1 min-w-0">
        <div className={`flex-shrink-0 ${statusConfig.color}`}>{statusConfig.icon}</div>
        <div className="ml-4 flex-1 min-w-0">
          <div className="flex items-center gap-2">
            <span className="text-sm font-medium text-gray-900">
              {isRefund ? 'Refund' : 'Payment'}
            </span>
            <TransactionStatusBadge status={transaction.status} />
          </div>
          <div className="mt-1 flex items-center gap-3 text-xs text-gray-600">
            {transaction.processedAt && (
              <span>{new Date(transaction.processedAt).toLocaleString()}</span>
            )}
            {transaction.paymentMethodInfo && (
              <PaymentMethodBadge paymentMethodInfo={transaction.paymentMethodInfo} />
            )}
          </div>
          {transaction.error && (
            <div className="mt-1 text-xs text-red-600">{transaction.error}</div>
          )}
        </div>
      </div>
      <div className="ml-4 flex-shrink-0">
        <span className={`text-sm font-semibold ${isRefund ? 'text-orange-600' : 'text-gray-900'}`}>
          {isRefund ? '-' : ''}
          {formatCurrency(Number(transaction.amount), currency)}
        </span>
      </div>
    </div>
  )
}

export const PaymentMethodBadge: React.FC<{
  paymentMethodInfo: Transaction['paymentMethodInfo']
}> = ({ paymentMethodInfo }) => {
  if (!paymentMethodInfo) return null

  const isCard = paymentMethodInfo.paymentMethodType === 0 // CARD

  if (isCard && paymentMethodInfo.cardBrand) {
    return (
      <div className="flex items-center gap-1">
        <CardBrandLogo brand={paymentMethodInfo.cardBrand} />
        {paymentMethodInfo.cardLast4 && <span>•••• {paymentMethodInfo.cardLast4}</span>}
      </div>
    )
  }

  if (paymentMethodInfo.paymentMethodType === 1) {
    // BANK_TRANSFER
    return <span>Bank Transfer</span>
  }

  return null
}

function getStatusConfig(status: Transaction_PaymentStatusEnum) {
  const configs: Record<Transaction_PaymentStatusEnum, { icon: React.ReactNode; color: string }> =
    {
      [Transaction_PaymentStatusEnum.READY]: {
        icon: <Clock className="h-5 w-5" />,
        color: 'text-blue-600',
      },
      [Transaction_PaymentStatusEnum.PENDING]: {
        icon: <Clock className="h-5 w-5" />,
        color: 'text-yellow-600',
      },
      [Transaction_PaymentStatusEnum.SETTLED]: {
        icon: <CheckCircle className="h-5 w-5" />,
        color: 'text-green-600',
      },
      [Transaction_PaymentStatusEnum.CANCELLED]: {
        icon: <Ban className="h-5 w-5" />,
        color: 'text-gray-500',
      },
      [Transaction_PaymentStatusEnum.FAILED]: {
        icon: <XCircle className="h-5 w-5" />,
        color: 'text-red-600',
      },
    }

  return configs[status] || configs[Transaction_PaymentStatusEnum.PENDING]
}
