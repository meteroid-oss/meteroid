import {
  Badge,
  Button,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '@md/ui'
import { MoreHorizontal, RefreshCwIcon } from 'lucide-react'
import { useState } from 'react'
import { Link } from 'react-router-dom'

import { PaymentMethodDisplay } from '@/features/invoices/PaymentMethodDisplay'
import { RefundCreditNoteOption, RefundDialog } from '@/features/invoices/RefundDialog'
import { ReversalKindBadge } from '@/features/invoices/ReversalKindBadge'
import { ReverseManualPaymentDialog } from '@/features/invoices/ReverseManualPaymentDialog'
import { TestGatewayActions } from '@/features/invoices/TestGatewayActions'
import { TransactionStatusBadge } from '@/features/invoices/TransactionStatusBadge'
import { amountFormat } from '@/features/invoices/amountFormat'
import {
  REFUND_MODE_LABELS,
  isRefundRow,
  isRefundableParent,
  refundableCents,
  refundsOf,
} from '@/features/invoices/refunds'
import { useBasePath } from '@/hooks/useBasePath'
import { formatCurrency } from '@/lib/utils/numbers'
import { Transaction, Transaction_PaymentStatusEnum } from '@/rpc/api/invoices/v1/models_pb'
import { parseAndFormatDate } from '@/utils/date'

interface TransactionListProps {
  transactions: Transaction[]
  currency: string
  isLoading?: boolean
  /** Finalized refund credit notes of the invoice a refund can draw on. */
  creditNoteOptions?: RefundCreditNoteOption[]
  /** The tenant has a payment connector that can give money back through the provider. */
  supportsRefunds?: boolean
  supportsPartialRefunds?: boolean
  /** Refund and reversal actions are backoffice-only. */
  showActions?: boolean
}

type RefundTarget = { transaction: Transaction; mode: 'online' | 'offline' }

export const TransactionList = ({
  transactions,
  currency,
  isLoading,
  creditNoteOptions = [],
  supportsRefunds = false,
  supportsPartialRefunds = false,
  showActions = false,
}: TransactionListProps) => {
  const [refundTarget, setRefundTarget] = useState<RefundTarget | null>(null)
  const [reverseTarget, setReverseTarget] = useState<Transaction | null>(null)

  if (isLoading) {
    return (
      <div className="flex items-center justify-center py-4">
        <RefreshCwIcon className="animate-spin mr-2" size={16} />
        <span className="text-sm text-muted-foreground">Loading transactions...</span>
      </div>
    )
  }

  if (!transactions?.length) {
    return <div className="text-sm text-muted-foreground py-2">No payment transactions</div>
  }

  const knownIds = new Set(transactions.map(tx => tx.id))
  // A refund row whose parent is not on this invoice still has to show up somewhere.
  const roots = transactions.filter(
    tx => !isRefundRow(tx) || !tx.parentTransactionId || !knownIds.has(tx.parentTransactionId)
  )

  return (
    <div className="space-y-2">
      {roots.map(transaction => (
        <div key={transaction.id} className="py-2">
          <TransactionRow
            transaction={transaction}
            transactions={transactions}
            currency={currency}
            showActions={showActions}
            supportsRefunds={supportsRefunds}
            onRefund={mode => setRefundTarget({ transaction, mode })}
            onReverse={() => setReverseTarget(transaction)}
          />
          {refundsOf(transaction, transactions).map(refund => (
            <div key={refund.id} className="ml-6 mt-2 border-l pl-3">
              <TransactionRow
                transaction={refund}
                transactions={transactions}
                currency={currency}
                showActions={false}
                supportsRefunds={false}
              />
            </div>
          ))}
        </div>
      ))}

      {refundTarget && (
        <RefundDialog
          open
          onOpenChange={() => setRefundTarget(null)}
          transaction={refundTarget.transaction}
          transactions={transactions}
          currency={currency}
          creditNoteOptions={creditNoteOptions}
          supportsOnlineRefunds={supportsRefunds}
          supportsPartialRefunds={supportsPartialRefunds}
          initialMode={refundTarget.mode}
        />
      )}

      {reverseTarget && (
        <ReverseManualPaymentDialog
          open
          onOpenChange={() => setReverseTarget(null)}
          transaction={reverseTarget}
          currency={currency}
        />
      )}
    </div>
  )
}

interface TransactionRowProps {
  transaction: Transaction
  transactions: Transaction[]
  currency: string
  showActions: boolean
  supportsRefunds: boolean
  onRefund?: (mode: 'online' | 'offline') => void
  onReverse?: () => void
}

const TransactionRow = ({
  transaction,
  transactions,
  currency,
  showActions,
  supportsRefunds,
  onRefund,
  onReverse,
}: TransactionRowProps) => {
  const basePath = useBasePath()
  const isRefund = isRefundRow(transaction)
  const refundable = isRefund ? 0n : refundableCents(transaction, transactions)
  const canRefund = showActions && isRefundableParent(transaction) && refundable > 0n
  const canReverse = showActions && isRefundableParent(transaction) && !transaction.paymentMethodId

  return (
    <div className="flex justify-between items-start">
      <div className="flex-1">
        <div className="flex items-center gap-2">
          <PaymentMethodDisplay paymentMethodInfo={transaction.paymentMethodInfo} compact />
          <TransactionStatusBadge status={transaction.status} transaction={transaction} />
          {isRefund && transaction.refundMode !== undefined && (
            <Badge variant="secondary" size="sm">
              {REFUND_MODE_LABELS[transaction.refundMode]}
            </Badge>
          )}
          {transaction.reversalKind !== undefined && (
            <ReversalKindBadge
              kind={transaction.reversalKind}
              reason={transaction.reversalReason}
            />
          )}
          {!transaction.paymentMethodId && transaction.providerTransactionId && (
            <span className="text-[11px] text-muted-foreground mt-1  ">
              ({transaction.providerTransactionId})
            </span>
          )}
        </div>
        <div className="text-[11px] text-muted-foreground mt-1  ">
          {transaction.status === Transaction_PaymentStatusEnum.FAILED
            ? transaction.error || 'Failed'
            : transaction.status === Transaction_PaymentStatusEnum.CANCELLED
              ? 'Cancelled'
              : transaction.processedAt
                ? parseAndFormatDate(transaction.processedAt)
                : 'Pending processing'}
        </div>
        {isRefund && transaction.creditNoteId && (
          <Link
            to={`${basePath}/credit-notes/${transaction.creditNoteId}`}
            className="text-[11px] text-brand hover:underline"
          >
            Credit note
          </Link>
        )}
        {!isRefund && transaction.amountRefunded > 0n && (
          <div className="text-[11px] text-muted-foreground mt-1">
            Refunded: {formatCurrency(transaction.amountRefunded, currency)}
          </div>
        )}
        {!isRefund && transaction.amountDisputed > 0n && (
          <div className="text-[11px] text-muted-foreground mt-1">
            Disputed: {formatCurrency(transaction.amountDisputed, currency)}
          </div>
        )}
        {!isRefund && transaction.amountReversed > transaction.amountDisputed && (
          <div className="text-[11px] text-muted-foreground mt-1">
            Clawed back:{' '}
            {formatCurrency(transaction.amountReversed - transaction.amountDisputed, currency)}
          </div>
        )}
        {!isRefund && transaction.amountReversedExcess > 0n && (
          <div className="text-[11px] text-warning mt-1">
            Withdrawn above what the payment held:{' '}
            {formatCurrency(transaction.amountReversedExcess, currency)} (contest with your
            provider)
          </div>
        )}
        <TestGatewayActions transaction={transaction} />
      </div>
      <div className="flex items-start gap-2">
        <div className="text-right">
          <div className="text-[13px] font-mono">
            {isRefund && '-'}
            {amountFormat({
              currency,
              total: transaction.amount,
            })}
          </div>
        </div>
        {(canRefund || canReverse) && (
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button variant="ghost" size="sm" className="h-6 w-6 p-0">
                <MoreHorizontal size={14} />
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end">
              {canRefund && supportsRefunds && transaction.paymentMethodId && (
                <DropdownMenuItem onClick={() => onRefund?.('online')}>Refund…</DropdownMenuItem>
              )}
              {canRefund && (
                <DropdownMenuItem onClick={() => onRefund?.('offline')}>
                  Record offline refund…
                </DropdownMenuItem>
              )}
              {canReverse && (
                <DropdownMenuItem
                  onClick={() => onReverse?.()}
                  className="text-destructive focus:text-destructive"
                >
                  Reverse…
                </DropdownMenuItem>
              )}
            </DropdownMenuContent>
          </DropdownMenu>
        )}
      </div>
    </div>
  )
}

export default TransactionList
