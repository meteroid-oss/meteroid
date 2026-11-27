import { useMutation } from '@connectrpc/connect-query'
import { Alert, Button } from '@md/ui'
import { CalendarClock } from 'lucide-react'
import { type ReactNode } from 'react'
import { toast } from 'sonner'

import { CopyToClipboardButton } from '@/components/CopyToClipboard'
import { Slot } from '@/edition'
import { plural } from '@/features/subscriptions/subscriptionOverview'
import { formatShortDay } from '@/features/subscriptions/utils/dates'
import { parseAndFormatDate, parseAndFormatDateTime } from '@/lib/utils/date'
import { formatCurrency } from '@/lib/utils/numbers'
import { ScheduledEventType, SubscriptionStatus } from '@/rpc/api/subscriptions/v1/models_pb'
import { cancelScheduledEvent } from '@/rpc/api/subscriptions/v1/subscriptions-SubscriptionsService_connectquery'

import type { PendingScheduledEvent, Subscription } from '@/rpc/api/subscriptions/v1/models_pb'
import type { Wallet } from '@/rpc/api/wallets/v1/models_pb'

export type BandVariant = 'default' | 'warning' | 'destructive'

/** One notice of the subscription's attention list. */
export const Band = ({
  variant,
  title,
  meta,
  children,
  actions,
}: {
  variant: BandVariant
  title: ReactNode
  meta?: ReactNode
  children?: ReactNode
  actions?: ReactNode
}) => (
  <Alert variant={variant}>
    <div className="flex flex-wrap items-start justify-between gap-3">
      <div className="flex min-w-0 flex-col gap-0.5">
        <span className="text-sm">
          <span className="font-semibold">{title}</span>
          {meta && <span className="text-foreground"> {meta}</span>}
        </span>
        {children && <span className="text-sm text-foreground">{children}</span>}
      </div>
      {actions && <div className="flex shrink-0 flex-wrap gap-2">{actions}</div>}
    </div>
  </Alert>
)

const scheduledEventLabel = (
  event: PendingScheduledEvent
): { title: string; variant: BandVariant } => {
  const on = parseAndFormatDate(event.scheduledDate)
  switch (event.eventType) {
    case ScheduledEventType.PLAN_CHANGE:
      return {
        title: `Plan change to ${event.newPlanName} scheduled for ${on}`,
        variant: 'default',
      }
    case ScheduledEventType.CANCEL:
      return {
        title: `Cancellation scheduled for ${on}${event.cancelReason ? ` (${event.cancelReason})` : ''}`,
        variant: 'destructive',
      }
    case ScheduledEventType.END_TRIAL:
      return { title: `Trial ending on ${on}`, variant: 'warning' }
    case ScheduledEventType.AMENDMENT:
      return { title: `Subscription amendment scheduled for ${on}`, variant: 'default' }
    default:
      return { title: `Scheduled event on ${on}`, variant: 'default' }
  }
}

const cancelLabel: Partial<Record<ScheduledEventType, string>> = {
  [ScheduledEventType.CANCEL]: 'Undo cancellation',
  [ScheduledEventType.PLAN_CHANGE]: 'Cancel change',
  [ScheduledEventType.AMENDMENT]: 'Cancel amendment',
}

const ScheduledEventBanner = ({
  event,
  subscriptionId,
  canManage,
  onCancelled,
}: {
  event: PendingScheduledEvent
  subscriptionId: string
  canManage: boolean
  onCancelled: () => void
}) => {
  const cancelMutation = useMutation(cancelScheduledEvent, {
    onSuccess: () => {
      toast.success(
        event.eventType === ScheduledEventType.CANCEL
          ? 'Cancellation undone: the subscription renews as usual'
          : 'Scheduled event cancelled'
      )
      onCancelled()
    },
    onError: (error: Error) => toast.error(`Failed to cancel: ${error.message}`),
  })
  const { title, variant } = scheduledEventLabel(event)
  const summary =
    event.eventType === ScheduledEventType.AMENDMENT ? event.amendmentSummary : undefined
  const lines = summary
    ? [
        ...summary.addedComponentNames.map(n => `+ ${n}`),
        ...summary.removedComponentNames.map(n => `− ${n}`),
        ...summary.addedAddOnNames.map(n => `+ ${n}`),
        ...summary.removedAddOnNames.map(n => `− ${n}`),
      ]
    : []

  return (
    <Band
      variant={variant}
      title={
        <span className="inline-flex items-center gap-2">
          <CalendarClock className="h-4 w-4 shrink-0" />
          {title}
        </span>
      }
      actions={
        canManage &&
        // A phase's or dated item's change goes with it: it is edited in the subscription's timeline.
        !event.timeline && (
          <Button
            size="sm"
            variant="outline"
            disabled={cancelMutation.isPending}
            onClick={() => cancelMutation.mutate({ eventId: event.id, subscriptionId })}
          >
            {cancelLabel[event.eventType] ?? 'Cancel scheduled event'}
          </Button>
        )
      }
    >
      {lines.length > 0 && (
        <span className="flex flex-col text-xs text-muted-foreground">
          {lines.map((line, i) => (
            <span key={i}>{line}</span>
          ))}
        </span>
      )}
    </Band>
  )
}

export type AttentionProps = {
  subscription: Subscription
  pendingEvents: PendingScheduledEvent[]
  wallets: Wallet[]
  firstInvoiceAmount?: bigint
  hasPaymentMethod: boolean
  canManageSubscription: boolean
  canManageWallets: boolean
  onAddCredit: (wallet: Wallet) => void
  onConfigureWallet: (wallet: Wallet) => void
  onEditBilling: () => void
  onRefetch: () => void
}

/** One line per cause needing action, most severe first; nothing when all is well. */
export const SubscriptionAttention = ({
  subscription: data,
  pendingEvents,
  wallets,
  firstInvoiceAmount,
  hasPaymentMethod,
  canManageSubscription,
  canManageWallets,
  onAddCredit,
  onConfigureWallet,
  onEditBilling,
  onRefetch,
}: AttentionProps) => {
  const money = (minor: bigint) => formatCurrency(minor, data.currency)
  const bands: ReactNode[] = []

  if (data.checkoutUrl && data.status === SubscriptionStatus.PENDING) {
    bands.push(
      <Band
        key="checkout"
        variant="default"
        title="This subscription is pending checkout."
        actions={
          <CopyToClipboardButton
            text="Copy checkout link"
            textToCopy={data.checkoutUrl}
            className="whitespace-normal"
          />
        }
      />
    )
  }

  if (data.errorCount > 0 && data.status !== SubscriptionStatus.ERRORED) {
    bands.push(
      <Band
        key="processing"
        variant="warning"
        title={`This subscription has encountered ${plural(data.errorCount, 'processing error')}.`}
      >
        {[
          data.lastError && `Last error: ${data.lastError}`,
          data.nextRetry && `Next retry: ${parseAndFormatDateTime(data.nextRetry)}`,
        ]
          .filter(Boolean)
          .join(' · ')}
      </Band>
    )
  }

  pendingEvents.forEach(event =>
    bands.push(
      <ScheduledEventBanner
        key={`event-${event.id}`}
        event={event}
        subscriptionId={data.id}
        canManage={canManageSubscription}
        onCancelled={onRefetch}
      />
    )
  )

  if (
    data.status === SubscriptionStatus.TRIALING &&
    data.chargeAutomatically &&
    !hasPaymentMethod
  ) {
    const trialEnd =
      pendingEvents.find(e => e.eventType === ScheduledEventType.END_TRIAL)?.scheduledDate ??
      data.currentPeriodEnd
    bands.push(
      <Band
        key="trial"
        variant="warning"
        title={trialEnd ? `Trial ends ${formatShortDay(trialEnd)}` : 'Trial ending'}
        meta={`· no payment method on file, so the first invoice${
          firstInvoiceAmount !== undefined ? ` (${money(firstInvoiceAmount)})` : ''
        } cannot be charged.`}
        actions={
          canManageSubscription &&
          (data.pendingCheckout && data.checkoutUrl ? (
            <CopyToClipboardButton text="Copy checkout link" textToCopy={data.checkoutUrl} />
          ) : (
            <Button size="sm" variant="outline" onClick={onEditBilling}>
              Edit billing settings
            </Button>
          ))
        }
      />
    )
  }

  return (
    <>
      <div className="mb-6 flex flex-col gap-2 empty:hidden">
        {data.status === SubscriptionStatus.ERRORED && (
          <Band
            variant="destructive"
            title={`This subscription failed to process after ${data.errorCount} attempts.`}
          >
            {data.lastError && `Last error: ${data.lastError}`}
          </Band>
        )}
        <Slot
          name="subscriptionDetail.notices"
          subscription={data}
          wallets={wallets}
          canManageWallets={canManageWallets}
          onAddCredit={onAddCredit}
          onConfigureWallet={onConfigureWallet}
        />
        {bands}
      </div>
      <div className="mb-6 empty:hidden">
        <Slot
          name="subscriptionDetail.attention"
          customerId={data.customerId}
          subscriptionId={data.id}
        />
      </div>
    </>
  )
}
