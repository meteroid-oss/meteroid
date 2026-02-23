import { useMutation } from '@connectrpc/connect-query'
import {
  Alert,
  DialogDescription,
  DialogTitle,
  Input,
  Label,
  Modal,
  RadioGroup,
  RadioGroupItem,
  Textarea,
} from '@md/ui'
import { AlertTriangle, XCircle } from 'lucide-react'
import { useState } from 'react'
import { toast } from 'sonner'

import { useQuery } from '@/lib/connectrpc'
import { ScheduledEventType } from '@/rpc/api/subscriptions/v1/models_pb'
import {
  cancelSubscription,
  getSubscriptionDetails,
} from '@/rpc/api/subscriptions/v1/subscriptions-SubscriptionsService_connectquery'
import {
  CancelSubscriptionRequest_BillingPeriodEnd,
  CancelSubscriptionRequest_Immediate,
} from '@/rpc/api/subscriptions/v1/subscriptions_pb'
import { parseAndFormatDate } from '@/utils/date'

interface CancelSubscriptionModalProps {
  subscriptionId: string
  customerName: string
  planName: string
  onClose: () => void
  onSuccess?: () => void
}

type CancellationTiming = 'immediate' | 'billing_period_end' | 'specific_date'

export const CancelSubscriptionModal = ({
  subscriptionId,
  customerName,
  planName,
  onClose,
  onSuccess,
}: CancelSubscriptionModalProps) => {
  const [reason, setReason] = useState('')
  const [timing, setTiming] = useState<CancellationTiming>('billing_period_end')
  const [specificDate, setSpecificDate] = useState('')

  const subscriptionQuery = useQuery(
    getSubscriptionDetails,
    { subscriptionId },
    { enabled: Boolean(subscriptionId) }
  )
  const pendingEvents = (subscriptionQuery.data?.pendingEvents ?? []).filter(
    e => e.eventType !== ScheduledEventType.CANCEL && e.eventType !== ScheduledEventType.END_TRIAL
  )

  const cancelMutation = useMutation(cancelSubscription, {
    onSuccess: () => {
      toast.success('Subscription scheduled for cancellation')
      onSuccess?.()
      onClose()
    },
    onError: error => {
      toast.error(
        `Failed to cancel subscription: ${error instanceof Error ? error.message : 'Unknown error'}`
      )
    },
  })

  const onConfirm = async () => {
    try {
      let effectiveAt:
        | { case: 'immediate'; value: CancelSubscriptionRequest_Immediate }
        | { case: 'billingPeriodEnd'; value: CancelSubscriptionRequest_BillingPeriodEnd }
        | { case: 'date'; value: string }
        | { case: undefined }

      switch (timing) {
        case 'immediate':
          effectiveAt = {
            case: 'immediate',
            value: new CancelSubscriptionRequest_Immediate(),
          }
          break
        case 'billing_period_end':
          effectiveAt = {
            case: 'billingPeriodEnd',
            value: new CancelSubscriptionRequest_BillingPeriodEnd(),
          }
          break
        case 'specific_date':
          if (!specificDate || new Date(specificDate) < new Date()) {
            toast.error('Please select a cancellation date in the future')
            return
          }
          effectiveAt = {
            case: 'date',
            value: specificDate,
          }
          break
        default:
          effectiveAt = { case: undefined }
      }

      await cancelMutation.mutateAsync({
        subscriptionId,
        reason: reason.trim() || undefined,
        effectiveAt,
      })
    } catch (error) {
      // Error already handled by onError
      console.error('Cancellation error:', error)
    }
  }

  return (
    <Modal
      header={
        <>
          <DialogTitle className="flex items-center gap-2 text-md">
            <XCircle className="w-6 h-6 text-destructive" />
            <span>Cancel Subscription</span>
          </DialogTitle>
          <DialogDescription className="text-sm">
            Cancel the subscription for {customerName} on plan {planName}.
          </DialogDescription>
        </>
      }
      visible={true}
      hideFooter={false}
      onCancel={onClose}
      onConfirm={onConfirm}
      confirmText={cancelMutation.isPending ? 'Cancelling...' : 'Cancel Subscription'}
      confirmDisabled={cancelMutation.isPending || (timing === 'specific_date' && !specificDate)}
    >
      <Modal.Content>
        <div className="space-y-4 py-4">
          {pendingEvents.length > 0 && (
            <Alert variant="warning">
              <div className="flex items-start gap-2">
                <AlertTriangle className="h-4 w-4 shrink-0 mt-0.5" />
                <div className="text-sm">
                  <span className="font-medium">
                    This will cancel the following pending events:
                  </span>
                  <ul className="mt-1 list-disc list-inside">
                    {pendingEvents.map(event => (
                      <li key={event.id}>
                        {event.eventType === ScheduledEventType.PLAN_CHANGE
                          ? `Plan change to "${event.newPlanName}"`
                          : event.eventType === ScheduledEventType.PAUSE
                            ? 'Pause'
                            : event.eventType === ScheduledEventType.END_TRIAL
                              ? 'Trial end'
                              : 'Event'}
                        {event.scheduledDate
                          ? ` on ${parseAndFormatDate(event.scheduledDate)}`
                          : ''}
                      </li>
                    ))}
                  </ul>
                </div>
              </div>
            </Alert>
          )}
          <div>
            <Label className="text-sm font-medium mb-3 block">
              When should the cancellation take effect?
            </Label>
            <RadioGroup
              value={timing}
              onValueChange={value => setTiming(value as CancellationTiming)}
            >
              <div className="flex items-center space-x-2 mb-2">
                <RadioGroupItem value="immediate" id="immediate" />
                <Label htmlFor="immediate" className="font-normal cursor-pointer">
                  Immediately
                </Label>
              </div>
              <div className="flex items-center space-x-2 mb-2">
                <RadioGroupItem value="billing_period_end" id="billing_period_end" />
                <Label htmlFor="billing_period_end" className="font-normal cursor-pointer">
                  End of the current billing periods
                </Label>
              </div>
              <div className="flex items-center space-x-2">
                <RadioGroupItem value="specific_date" id="specific_date" />
                <Label htmlFor="specific_date" className="font-normal cursor-pointer">
                  Specific date
                </Label>
              </div>
            </RadioGroup>
          </div>

          {timing === 'specific_date' && (
            <div>
              <Label htmlFor="cancellation-date" className="text-sm font-medium mb-2 block">
                Cancellation Date
              </Label>
              <Input
                id="cancellation-date"
                type="date"
                value={specificDate}
                onChange={e => setSpecificDate(e.target.value)}
                min={new Date().toISOString().split('T')[0]}
              />
            </div>
          )}

          <div>
            <Label htmlFor="reason" className="text-sm font-medium mb-2 block">
              Cancellation Reason (Optional)
            </Label>
            <Textarea
              id="reason"
              placeholder="Enter the reason for cancellation..."
              value={reason}
              onChange={e => setReason(e.target.value)}
              rows={3}
            />
          </div>
        </div>
      </Modal.Content>
    </Modal>
  )
}
