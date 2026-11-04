import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query'
import { Button } from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { FlaskConical } from 'lucide-react'
import { toast } from 'sonner'

import { testGatewayActionsFor } from '@/features/payments/testGateway'
import {
  getInvoice,
  simulateTestGatewayEvent,
} from '@/rpc/api/invoices/v1/invoices-InvoicesService_connectquery'
import { Transaction } from '@/rpc/api/invoices/v1/models_pb'
import { useConfirmationModal } from 'providers/ConfirmationProvider'

/**
 * The provider events a merchant would otherwise wait for (or post by hand) on a test-gateway
 * transaction. Renders nothing for real providers.
 */
export const TestGatewayActions = ({ transaction }: { transaction: Transaction }) => {
  const queryClient = useQueryClient()
  const simulate = useMutation(simulateTestGatewayEvent, {
    onSuccess: () =>
      queryClient.invalidateQueries({
        queryKey: createConnectQueryKey({ schema: getInvoice, cardinality: undefined }),
      }),
    onError: err => toast.error(err.rawMessage || 'The simulated event was refused'),
  })

  const showConfirmModal = useConfirmationModal()
  const actions = testGatewayActionsFor(transaction)
  if (actions.length === 0) return null

  const run = (action: (typeof actions)[number]) => {
    const mutate = () => simulate.mutate({ transactionId: transaction.id, event: action.event })
    if (!action.confirm) return mutate()
    showConfirmModal(mutate, { header: action.label, message: action.confirm })
  }

  return (
    <div className="flex items-center gap-1 text-[11px] text-muted-foreground mt-1">
      <FlaskConical size={12} className="shrink-0" />
      <span>Test gateway:</span>
      {actions.map(action => (
        <Button
          key={action.event}
          type="button"
          variant="ghost"
          size="sm"
          className="h-5 px-1.5 text-[11px]"
          disabled={simulate.isPending}
          onClick={() => run(action)}
        >
          {action.label}
        </Button>
      ))}
    </div>
  )
}
