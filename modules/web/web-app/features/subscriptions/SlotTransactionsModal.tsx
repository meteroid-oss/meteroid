import { useMutation, useQuery } from '@connectrpc/connect-query'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
  Tabs,
  TabsContent,
  TabsList,
  TabsTrigger,
} from '@md/ui'
import { format } from 'date-fns'
import { AlertCircle, ArrowDown, ArrowUp, XCircle } from 'lucide-react'
import { Link } from 'react-router-dom'
import { toast } from 'sonner'

import { useBasePath } from '@/hooks/useBasePath'
import { SlotTransactionStatus } from '@/rpc/api/subscriptions/v1/models_pb'
import {
  cancelSlotTransaction,
  listSlotTransactions,
} from '@/rpc/api/subscriptions/v1/subscriptions-SubscriptionsService_connectquery'

import type { SlotTransaction } from '@/rpc/api/subscriptions/v1/models_pb'

interface SlotTransactionsModalProps {
  subscriptionId: string
  unit: string
  open: boolean
  onClose: () => void
}

export const SlotTransactionsModal = ({
  subscriptionId,
  unit,
  open,
  onClose,
}: SlotTransactionsModalProps) => {
  const transactionsQuery = useQuery(
    listSlotTransactions,
    {
      subscriptionId,
      unit,
    },
    {
      enabled: open,
    }
  )

  const cancelMutation = useMutation(cancelSlotTransaction, {
    onSuccess: () => {
      toast.success('Slot change cancelled')
      transactionsQuery.refetch()
    },
    onError: error => {
      toast.error(`Failed to cancel: ${error instanceof Error ? error.message : 'Unknown error'}`)
    },
  })

  const handleCancel = (transactionId: string, delta: number) => {
    const action = delta > 0 ? 'upgrade' : 'downgrade'
    if (confirm(`Are you sure you want to cancel this ${unit} ${action}?`)) {
      cancelMutation.mutate({ transactionId })
    }
  }

  const transactions = transactionsQuery.data?.transactions ?? []
  const now = new Date()

  // Upcoming: ACTIVE (future) + PENDING
  const upcomingTransactions = transactions.filter(t => {
    if (t.delta === 0) return false
    if (t.status === SlotTransactionStatus.SLOT_PENDING) return true
    if (t.status === SlotTransactionStatus.SLOT_ACTIVE && t.effectiveAt) {
      return new Date(t.effectiveAt) >= now
    }
    return false
  })

  // History: ACTIVE (past)
  const historyTransactions = transactions.filter(t => {
    if (t.delta === 0) return false
    if (t.status === SlotTransactionStatus.SLOT_ACTIVE && t.effectiveAt) {
      return new Date(t.effectiveAt) < now
    }
    return false
  })

  return (
    <Dialog open={open} onOpenChange={onClose}>
      <DialogContent className="max-w-2xl max-h-[80vh] overflow-hidden flex flex-col">
        <DialogHeader>
          <DialogTitle>Slot Transactions</DialogTitle>
          <DialogDescription>
            View and manage {unit} changes for this subscription
          </DialogDescription>
        </DialogHeader>

        <Tabs defaultValue="upcoming" className="flex-1 overflow-hidden flex flex-col">
          <TabsList className="grid w-full grid-cols-2">
            <TabsTrigger value="upcoming">
              Upcoming {upcomingTransactions.length > 0 && `(${upcomingTransactions.length})`}
            </TabsTrigger>
            <TabsTrigger value="history">
              History {historyTransactions.length > 0 && `(${historyTransactions.length})`}
            </TabsTrigger>
          </TabsList>

          <TabsContent value="upcoming" className="flex-1 overflow-auto mt-4">
            {upcomingTransactions.length === 0 ? (
              <div className="text-center py-12 text-muted-foreground">No upcoming changes</div>
            ) : (
              <div className="border rounded-lg">
                <table className="w-full">
                  <thead className="border-b bg-muted/30 hidden">
                    <tr>
                      <th className="px-4 py-3 text-left text-xs font-medium text-muted-foreground">
                        Change
                      </th>
                      <th className="px-4 py-3 text-left text-xs font-medium text-muted-foreground">
                        Effective
                      </th>
                      <th className="px-4 py-3 text-left text-xs font-medium text-muted-foreground">
                        Status
                      </th>
                      <th className="px-4 py-3 text-right text-xs font-medium text-muted-foreground w-16"></th>
                    </tr>
                  </thead>
                  <tbody>
                    {upcomingTransactions.map(transaction => (
                      <TransactionRow
                        key={transaction.id}
                        transaction={transaction}
                        onCancel={handleCancel}
                        cancelDisabled={cancelMutation.isPending}
                        isCancellable={
                          transaction.status === SlotTransactionStatus.SLOT_ACTIVE &&
                          new Date(transaction.effectiveAt) >= now
                        }
                      />
                    ))}
                  </tbody>
                </table>
              </div>
            )}
          </TabsContent>

          <TabsContent value="history" className="flex-1 overflow-auto mt-4">
            {historyTransactions.length === 0 ? (
              <div className="text-center py-12 text-muted-foreground">No history yet</div>
            ) : (
              <div className="border rounded-lg">
                <table className="w-full">
                  <thead className="border-b bg-muted/30 hidden">
                    <tr>
                      <th className="px-4 py-3 text-left text-xs font-medium text-muted-foreground">
                        Change
                      </th>
                      <th className="px-4 py-3 text-left text-xs font-medium text-muted-foreground">
                        Effective
                      </th>
                      <th className="px-4 py-3 text-left text-xs font-medium text-muted-foreground">
                        Status
                      </th>
                      <th className="px-4 py-3 text-right text-xs font-medium text-muted-foreground w-16"></th>
                    </tr>
                  </thead>
                  <tbody>
                    {historyTransactions.map(transaction => (
                      <TransactionRow
                        key={transaction.id}
                        transaction={transaction}
                        onCancel={handleCancel}
                        cancelDisabled={false}
                        isCancellable={false}
                      />
                    ))}
                  </tbody>
                </table>
              </div>
            )}
          </TabsContent>
        </Tabs>
      </DialogContent>
    </Dialog>
  )
}

interface TransactionRowProps {
  transaction: SlotTransaction
  onCancel: (id: string, delta: number) => void
  cancelDisabled: boolean
  isCancellable: boolean
}

const TransactionRow = ({
  transaction,
  onCancel,
  cancelDisabled,
  isCancellable,
}: TransactionRowProps) => {
  const isUpgrade = transaction.delta > 0
  const isPending = transaction.status === SlotTransactionStatus.SLOT_PENDING

  const basePath = useBasePath()

  return (
    <tr className="border-b last:border-0 hover:bg-muted/10">
      <td className="px-4 py-3">
        <div className="flex items-center gap-2">
          {isUpgrade ? (
            <ArrowUp size={16} className="text-success flex-shrink-0" />
          ) : (
            <ArrowDown size={16} className="text-warning flex-shrink-0" />
          )}
          <div>
            <div className="text-sm font-medium">
              {transaction.prevActiveSlots} → {transaction.newActiveSlots} ({isUpgrade ? '+' : ''}
              {transaction.delta})
            </div>
          </div>
        </div>
      </td>
      <td className="px-4 py-3">
        <div className="text-sm">
          {transaction.effectiveAt ? format(new Date(transaction.effectiveAt), 'PPP') : 'N/A'}
        </div>
      </td>
      <td className="px-4 py-3">
        <div className="flex items-center gap-1.5">
          {isPending ? (
            <>
              <AlertCircle size={14} className="text-warning" />
              <span className="text-xs text-muted-foreground">Pending payment</span>
            </>
          ) : (
            <span className="text-xs text-success">
              {transaction.effectiveAt && new Date(transaction.effectiveAt) < new Date()
                ? 'Applied'
                : 'Scheduled'}
            </span>
          )}
        </div>
        {transaction.invoiceId && (
          <div className="text-xs text-muted-foreground mt-0.5">
            <Link
              to={`${basePath}/invoices/${transaction.invoiceId}`}
              className="underline hover:text-foreground"
            >
              Invoice {transaction.invoiceId.slice(0, 8)}
            </Link>
          </div>
        )}
      </td>
      <td className="px-4 py-3 text-right">
        {isCancellable && (
          <button
            onClick={() => onCancel(transaction.id, transaction.delta)}
            disabled={cancelDisabled}
            className="p-1.5 text-muted-foreground hover:text-destructive hover:bg-destructive/10 rounded transition-colors disabled:opacity-50"
            title="Cancel change"
          >
            <XCircle size={16} />
          </button>
        )}
      </td>
    </tr>
  )
}
