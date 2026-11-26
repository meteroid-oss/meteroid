import { useMutation } from '@connectrpc/connect-query'
import { toast } from 'sonner'

import { slotItems } from '@/edition'
import { cancelSubscription } from '@/rpc/portal/subscription/v1/subscription-PortalSubscriptionService_connectquery'

import { date } from '../format'
import { useInvalidatePortal } from '../hooks'
import { Modal, ModalCloseButton, PButton } from '../primitives'
import { customerError } from '../walletFormat'

import type { CancelFlowOutcome } from '@/edition/types'

interface CancelSubscriptionProps {
  open: boolean
  onClose: () => void
  onDone: (outcome: CancelFlowOutcome, effectiveDate?: string) => void
  subscriptionId: string
  planName: string
  periodEnd?: string
}

/** The module's cancel flow when one is installed, else the plain confirmation. */
export const CancelSubscription = (props: CancelSubscriptionProps) => {
  const [Flow] = slotItems('portal.subscription.cancelFlow')
  const fallback = <PlainCancel {...props} />
  return Flow ? <Flow {...props} fallback={fallback} /> : fallback
}

const PlainCancel = ({
  open,
  onClose,
  onDone,
  subscriptionId,
  planName,
  periodEnd,
}: CancelSubscriptionProps) => {
  const cancel = useMutation(cancelSubscription)
  const invalidatePortal = useInvalidatePortal()
  const confirm = async () => {
    try {
      await cancel.mutateAsync({ subscriptionId, immediate: false })
      await invalidatePortal()
      toast.success(
        periodEnd ? `Your subscription ends on ${date(periodEnd)}.` : 'Cancellation scheduled'
      )
      onDone('cancelled', periodEnd)
    } catch (e) {
      toast.error(customerError(e, 'Unable to cancel'))
    }
  }
  return (
    <Modal open={open} onClose={onClose} maxWidth={420}>
      <div style={{ padding: 24, display: 'flex', flexDirection: 'column', gap: 14 }}>
        <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
          <span style={{ fontSize: 16, fontWeight: 600 }}>
            Cancel your {planName} subscription?
          </span>
          <ModalCloseButton onClose={onClose} />
        </div>
        <p style={{ fontSize: 13, color: 'var(--mtp-text-2)', lineHeight: 1.5, margin: 0 }}>
          {periodEnd
            ? `Your subscription stays active until ${date(periodEnd)}. You can undo this until then.`
            : 'Your subscription stays active until the end of the period. You can undo this until then.'}
        </p>
        <div style={{ display: 'flex', justifyContent: 'flex-end', gap: 9, marginTop: 4 }}>
          <PButton variant="secondary" onClick={onClose}>
            Keep subscription
          </PButton>
          <PButton variant="danger" onClick={confirm} loading={cancel.isPending}>
            Cancel subscription
          </PButton>
        </div>
      </div>
    </Modal>
  )
}
