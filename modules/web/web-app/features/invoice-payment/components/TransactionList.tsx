import { Ban, CheckCircle, Clock, XCircle } from 'lucide-react'

import { CardBrandLogo } from '@/features/checkout/components/CardBrandLogo'
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
            <span
              className={`inline-flex items-center px-2 py-0.5 rounded text-xs font-medium ${statusConfig.badge}`}
            >
              {statusConfig.label}
            </span>
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
  const configs: Record<
    Transaction_PaymentStatusEnum,
    {
      icon: React.ReactNode
      color: string
      label: string
      badge: string
    }
  > = {
    [Transaction_PaymentStatusEnum.READY]: {
      icon: <Clock className="h-5 w-5" />,
      color: 'text-blue-600',
      label: 'Ready',
      badge: 'bg-blue-100 text-blue-800',
    },
    [Transaction_PaymentStatusEnum.PENDING]: {
      icon: <Clock className="h-5 w-5" />,
      color: 'text-yellow-600',
      label: 'Pending',
      badge: 'bg-yellow-100 text-yellow-800',
    },
    [Transaction_PaymentStatusEnum.SETTLED]: {
      icon: <CheckCircle className="h-5 w-5" />,
      color: 'text-green-600',
      label: 'Settled',
      badge: 'bg-green-100 text-green-800',
    },
    [Transaction_PaymentStatusEnum.CANCELLED]: {
      icon: <Ban className="h-5 w-5" />,
      color: 'text-gray-500',
      label: 'Cancelled',
      badge: 'bg-gray-100 text-gray-800',
    },
    [Transaction_PaymentStatusEnum.FAILED]: {
      icon: <XCircle className="h-5 w-5" />,
      color: 'text-red-600',
      label: 'Failed',
      badge: 'bg-red-100 text-red-800',
    },
  }

  return configs[status] || configs[Transaction_PaymentStatusEnum.PENDING]
}
