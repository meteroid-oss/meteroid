import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query'
import { Button, HoverCard, HoverCardContent, HoverCardTrigger } from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { formatDistanceToNow } from 'date-fns'
import {
  Activity,
  Archive,
  ArchiveRestore,
  Bell,
  BellMinus,
  BellOff,
  BellRing,
  CalendarX,
  CheckCircle2,
  CirclePause,
  CircleX,
  Eye,
  FileText,
  KeyRound,
  Link2,
  Link2Off,
  LogIn,
  Mail,
  MailQuestion,
  MailX,
  OctagonX,
  PencilLine,
  Play,
  Plus,
  Receipt,
  RotateCcw,
  Send,
  ShieldCheck,
  ToggleLeft,
  ToggleRight,
  Trash2,
  Users,
  XCircle,
} from 'lucide-react'
import { ReactNode, useState } from 'react'
import { Link } from 'react-router-dom'
import { toast } from 'sonner'

import { Gate } from '@/components/Gate'
import { EmailPreviewDialog } from '@/features/customer-emails/EmailPreviewDialog'
import { offsetPill } from '@/features/customer-emails/offsetPill'
import { skipReason } from '@/features/customer-emails/skipReasons'
import { useBasePath } from '@/hooks/useBasePath'
import { parseAndFormatDateTime } from '@/lib/utils/date'
import { formatCurrency } from '@/lib/utils/numbers'
import { listActivity } from '@/rpc/api/activity/v1/activity-ActivityFeedService_connectquery'
import { listEntityActivity } from '@/rpc/api/activity/v1/activity-ActivityService_connectquery'
import { ActivityEntry, ActorType } from '@/rpc/api/activity/v1/activity_pb'
import {
  listSentEmails,
  resendUnknownEmail,
} from '@/rpc/api/customeremails/v1/customeremails-CustomerEmailsService_connectquery'
import { useConfirmationModal } from 'providers/ConfirmationProvider'

import { EntityHoverCard } from './EntityHoverCard'
import { parseMetadata } from './types'

type Props = {
  entry: ActivityEntry
  compact?: boolean
}

interface RenderedEntry {
  icon: ReactNode
  title: ReactNode
  subtitle?: ReactNode
  emailPreview?: { activityId: string }
  // A delivery-unknown email an operator may send again (not reminders: they have "Send now").
  sendAgain?: { activityId: string; kind: string }
}

function actorByline(entry: ActivityEntry): ReactNode {
  const label = actorLabel(entry)
  if (entry.actorType === ActorType.SYSTEM) {
    return <span className="text-muted-foreground/60">{label}</span>
  }
  if (entry.actorType === ActorType.CUSTOMER && entry.actorId) {
    return (
      <EntityHoverCard
        entityType="customer"
        entityId={entry.actorId}
        label={label}
        className="font-medium text-foreground/80"
      />
    )
  }
  return <span className="font-medium text-foreground/80">{label}</span>
}

function actorLabel(entry: ActivityEntry): string {
  if (entry.actorName) return entry.actorName
  switch (entry.actorType) {
    case ActorType.SYSTEM:
      return 'System'
    case ActorType.USER:
      return 'User'
    case ActorType.API_TOKEN:
      return 'API token'
    case ActorType.CUSTOMER:
      return 'Customer'
    case ActorType.QUOTE_RECIPIENT:
      return entry.actorId ?? 'Recipient'
    case ActorType.OAUTH_APP:
      return entry.actorName ?? 'OAuth app'
    default:
      return 'Unknown'
  }
}

function entityRef(entry: ActivityEntry, prefix: string, fallback: string): ReactNode {
  const named = entry.entityName ? `${prefix} ${entry.entityName}` : fallback
  return (
    <EntityHoverCard
      entityType={entry.entityType}
      entityId={entry.entityId}
      label={entry.entityDeleted ? `${named} (deleted)` : named}
      deleted={entry.entityDeleted}
    />
  )
}

// The row's own verb already says the entity is gone, so it skips the "(deleted)" suffix
// `entityRef` adds — but still renders as copy-the-id text rather than a dead link.
function goneRef(entry: ActivityEntry, label: string): ReactNode {
  return (
    <EntityHoverCard
      entityType={entry.entityType}
      entityId={entry.entityId}
      label={label}
      deleted
    />
  )
}

// Wire email kind (`payment_reminder`) → sentence-leading label (`Payment reminder`).
function emailKindName(kind: string): string {
  const words = kind.replace(/_/g, ' ')
  return words.charAt(0).toUpperCase() + words.slice(1)
}

// Human label for a raw entity_type string (e.g. `invoicing_entity` → `Invoicing entity`),
// so generic `entity.updated`/`entity.field_changed` events name the entity they touched.
function entityTypeLabel(entityType: string): string {
  switch (entityType) {
    case 'invoicing_entity':
      return 'Invoicing entity'
    case 'tenant':
      return 'Tenant'
    case 'plan':
      return 'Plan'
    case 'customer':
      return 'Customer'
    case 'product':
      return 'Product'
    case 'coupon':
      return 'Coupon'
    case 'add_on':
      return 'Add-on'
    case 'billable_metric':
      return 'Billable metric'
    case 'invoice':
      return 'Invoice'
    case 'credit_note':
      return 'Credit note'
    case 'subscription':
      return 'Subscription'
    case 'quote':
      return 'Quote'
    case 'payment_reminder_sequence':
      return 'Reminder sequence'
    default:
      return entityType.replace(/_/g, ' ')
  }
}

// Sequences have no page of their own: link to the sequence sheet in the settings tab.
const SequenceLink = ({ id, name }: { id: string; name?: string }) => {
  const basePath = useBasePath()
  return (
    <Link
      to={`${basePath}/settings/emails?view=reminders&sequence=${id}`}
      className="text-foreground underline-offset-2 hover:underline"
    >
      {name ? `Reminder sequence "${name}"` : 'Reminder sequence'}
    </Link>
  )
}

const humanize = (value: unknown): string | undefined =>
  typeof value === 'string' ? value.replace(/_/g, ' ') : undefined

const optionalDateTime = (value: unknown): string | undefined =>
  typeof value === 'string' && value ? parseAndFormatDateTime(value) : undefined

function reminderStepSubtitle(md: Record<string, unknown>): string | undefined {
  const parts: string[] = []
  if (typeof md.sequence_name === 'string') parts.push(`Sequence "${md.sequence_name}"`)
  const skipped = Array.isArray(md.skipped_offsets) ? (md.skipped_offsets as number[]) : []
  if (skipped.length > 0) parts.push(`skipped ${skipped.map(offsetPill).join(', ')}`)
  const next = optionalDateTime(md.next_reminder_at)
  parts.push(next ? `next ${next}` : 'no further reminders')
  return parts.join(' · ')
}

function recipientsNode(recipients: string[]): ReactNode {
  if (recipients.length === 0) return null
  const summary = `${recipients.length} recipient${recipients.length === 1 ? '' : 's'}`
  return (
    <HoverCard openDelay={150}>
      <HoverCardTrigger asChild>
        <span className="underline underline-offset-2 cursor-help">{summary}</span>
      </HoverCardTrigger>
      <HoverCardContent className="w-72 p-3" align="start">
        <div className="text-xs font-medium mb-1">Sent to</div>
        <ul className="text-xs text-muted-foreground space-y-1">
          {recipients.map(r => (
            <li key={r} className="break-all">
              {r}
            </li>
          ))}
        </ul>
      </HoverCardContent>
    </HoverCard>
  )
}

function renderEntry(entry: ActivityEntry): RenderedEntry {
  const md = parseMetadata(entry)

  switch (entry.activityType) {
    case 'customer.created':
      return {
        icon: <Plus className="h-4 w-4 text-success" />,
        title: <>{entityRef(entry, 'Customer', 'Customer')} created</>,
      }
    case 'customer.updated':
      return {
        icon: <PencilLine className="h-4 w-4" />,
        title: <>{entityRef(entry, 'Customer', 'Customer')} updated</>,
      }
    case 'customer.archived':
      return {
        icon: <Archive className="h-4 w-4 text-warning" />,
        title: <>{entityRef(entry, 'Customer', 'Customer')} archived</>,
      }
    case 'customer.unarchived':
      return {
        icon: <Archive className="h-4 w-4 text-success" />,
        title: <>{entityRef(entry, 'Customer', 'Customer')} unarchived</>,
      }
    case 'customer.logged_in':
      return {
        icon: <LogIn className="h-4 w-4" />,
        title: <>{entityRef(entry, 'Customer', 'Customer')} signed in to the portal</>,
      }

    case 'invoice.created':
      return {
        icon: <FileText className="h-4 w-4 text-success" />,
        title: <>{entityRef(entry, 'Invoice', 'Invoice')} created</>,
      }
    case 'invoice.finalized':
      return {
        icon: <CheckCircle2 className="h-4 w-4 text-foreground" />,
        title: <>{entityRef(entry, 'Invoice', 'Invoice')} finalized</>,
      }
    case 'invoice.paid':
      return {
        icon: <Receipt className="h-4 w-4 text-success" />,
        title: <>{entityRef(entry, 'Invoice', 'Invoice')} marked as paid</>,
      }
    case 'invoice.voided':
      return {
        icon: <CircleX className="h-4 w-4 text-destructive" />,
        title: <>{entityRef(entry, 'Invoice', 'Invoice')} voided</>,
      }
    case 'invoice.deleted': {
      const total = typeof md.total === 'number' ? md.total : undefined
      const currency = typeof md.currency === 'string' ? md.currency : undefined
      return {
        icon: <Trash2 className="h-4 w-4 text-destructive" />,
        title: <>{goneRef(entry, 'Draft invoice')} deleted</>,
        subtitle: total !== undefined && currency ? formatCurrency(total, currency) : undefined,
      }
    }
    case 'invoice.consolidated': {
      const parentId =
        typeof md.consolidated_into_invoice_id === 'string'
          ? md.consolidated_into_invoice_id
          : undefined
      return {
        icon: <FileText className="h-4 w-4 text-muted-foreground" />,
        title: (
          <>
            {entityRef(entry, 'Invoice', 'Invoice')} merged into{' '}
            {parentId ? (
              <EntityHoverCard
                entityType="invoice"
                entityId={parentId}
                label="a consolidated invoice"
              />
            ) : (
              'a consolidated invoice'
            )}
          </>
        ),
      }
    }

    case 'product.created':
      return {
        icon: <Plus className="h-4 w-4 text-success" />,
        title: <>{entityRef(entry, 'Product', 'Product')} created</>,
      }
    case 'product.updated':
      return {
        icon: <PencilLine className="h-4 w-4" />,
        title: <>{entityRef(entry, 'Product', 'Product')} updated</>,
      }
    case 'product.archived':
      return {
        icon: <Archive className="h-4 w-4 text-warning" />,
        title: <>{entityRef(entry, 'Product', 'Product')} archived</>,
      }

    case 'plan.created':
      return {
        icon: <Plus className="h-4 w-4 text-success" />,
        title: <>{entityRef(entry, 'Plan', 'Plan')} created</>,
      }
    case 'plan.published':
      return {
        icon: <Send className="h-4 w-4 text-foreground" />,
        title: <>{entityRef(entry, 'Plan', 'Plan')} new version published</>,
      }
    case 'plan.archived':
      return {
        icon: <Archive className="h-4 w-4 text-warning" />,
        title: <>{entityRef(entry, 'Plan', 'Plan')} archived</>,
      }
    case 'plan.draft_discarded':
      return {
        icon: <XCircle className="h-4 w-4 text-warning" />,
        title: <>{entityRef(entry, 'Plan', 'Plan')} draft discarded</>,
      }

    case 'add_on.created':
      return {
        icon: <Plus className="h-4 w-4 text-success" />,
        title: <>{entityRef(entry, 'Add-on', 'Add-on')} created</>,
      }
    case 'add_on.updated':
      return {
        icon: <PencilLine className="h-4 w-4" />,
        title: <>{entityRef(entry, 'Add-on', 'Add-on')} updated</>,
      }
    case 'add_on.archived':
      return {
        icon: <Archive className="h-4 w-4 text-warning" />,
        title: <>{entityRef(entry, 'Add-on', 'Add-on')} archived</>,
      }

    case 'coupon.created':
      return {
        icon: <Plus className="h-4 w-4 text-success" />,
        title: <>{entityRef(entry, 'Coupon', 'Coupon')} created</>,
      }
    case 'coupon.updated':
      return {
        icon: <PencilLine className="h-4 w-4" />,
        title: <>{entityRef(entry, 'Coupon', 'Coupon')} updated</>,
      }
    case 'coupon.archived':
      return {
        icon: <Archive className="h-4 w-4 text-warning" />,
        title: <>{entityRef(entry, 'Coupon', 'Coupon')} archived</>,
      }

    case 'feature.created':
      return {
        icon: <Plus className="h-4 w-4 text-success" />,
        title: <>{entityRef(entry, 'Feature', 'Feature')} created</>,
      }
    case 'feature.enabled':
      return {
        icon: <ToggleRight className="h-4 w-4 text-success" />,
        title: <>{entityRef(entry, 'Feature', 'Feature')} enabled</>,
      }
    case 'feature.disabled':
      return {
        icon: <ToggleLeft className="h-4 w-4 text-warning" />,
        title: <>{entityRef(entry, 'Feature', 'Feature')} disabled</>,
      }
    case 'feature.archived':
      return {
        icon: <Archive className="h-4 w-4 text-warning" />,
        title: <>{entityRef(entry, 'Feature', 'Feature')} archived</>,
      }
    case 'feature.unarchived':
      return {
        icon: <ArchiveRestore className="h-4 w-4 text-success" />,
        title: <>{entityRef(entry, 'Feature', 'Feature')} unarchived</>,
      }

    case 'billable_metric.created':
      return {
        icon: <Plus className="h-4 w-4 text-success" />,
        title: <>{entityRef(entry, 'Metric', 'Metric')} created</>,
      }
    case 'billable_metric.updated':
      return {
        icon: <PencilLine className="h-4 w-4" />,
        title: <>{entityRef(entry, 'Metric', 'Metric')} updated</>,
      }
    case 'billable_metric.archived':
      return {
        icon: <Archive className="h-4 w-4 text-warning" />,
        title: <>{entityRef(entry, 'Metric', 'Metric')} archived</>,
      }

    case 'subscription.created':
      return {
        icon: <Plus className="h-4 w-4 text-success" />,
        title: <>{entityRef(entry, 'Subscription on', 'Subscription')} created</>,
      }

    case 'credit_note.created':
      return {
        icon: <FileText className="h-4 w-4 text-success" />,
        title: <>{entityRef(entry, 'Credit note', 'Credit note')} created</>,
      }
    case 'credit_note.finalized':
      return {
        icon: <CheckCircle2 className="h-4 w-4 text-foreground" />,
        title: <>{entityRef(entry, 'Credit note', 'Credit note')} finalized</>,
      }
    case 'credit_note.voided':
      return {
        icon: <XCircle className="h-4 w-4 text-destructive" />,
        title: <>{entityRef(entry, 'Credit note', 'Credit note')} voided</>,
      }

    case 'quote.created':
      return {
        icon: <Plus className="h-4 w-4 text-success" />,
        title: <>{entityRef(entry, 'Quote', 'Quote')} created</>,
      }
    case 'quote.updated':
      return {
        icon: <PencilLine className="h-4 w-4" />,
        title: <>{entityRef(entry, 'Quote', 'Quote')} updated</>,
      }
    case 'quote.accepted':
      return {
        icon: <CheckCircle2 className="h-4 w-4 text-success" />,
        title: <>{entityRef(entry, 'Quote', 'Quote')} accepted</>,
      }
    case 'quote.converted':
      return {
        icon: <CheckCircle2 className="h-4 w-4 text-success" />,
        title: <>{entityRef(entry, 'Quote', 'Quote')} converted to subscription</>,
      }
    case 'quote.published':
      return {
        icon: <Send className="h-4 w-4 text-foreground" />,
        title: <>{entityRef(entry, 'Quote', 'Quote')} published</>,
      }
    case 'quote.sent': {
      const recipients = Array.isArray(md.recipients) ? (md.recipients as string[]) : []
      return {
        icon: <Mail className="h-4 w-4" />,
        title: (
          <>
            {entityRef(entry, 'Quote', 'Quote')} sent
            {recipients.length > 0 && <> to {recipientsNode(recipients)}</>}
          </>
        ),
      }
    }
    case 'quote.declined':
      return {
        icon: <XCircle className="h-4 w-4 text-destructive" />,
        title: <>{entityRef(entry, 'Quote', 'Quote')} declined</>,
        subtitle: typeof md.reason === 'string' ? md.reason : undefined,
      }
    case 'quote.cancelled':
      return {
        icon: <XCircle className="h-4 w-4 text-destructive" />,
        title: <>{entityRef(entry, 'Quote', 'Quote')} cancelled</>,
        subtitle: typeof md.reason === 'string' ? md.reason : undefined,
      }
    case 'quote.viewed':
      return {
        icon: <Eye className="h-4 w-4 text-muted-foreground" />,
        title: <>{entityRef(entry, 'Quote', 'Quote')} viewed by recipient</>,
      }
    case 'quote.signature_added': {
      const name = (md.signed_by_name as string | undefined) ?? entry.actorName ?? entry.actorId
      const ip = typeof md.signed_from_ip === 'string' ? md.signed_from_ip : undefined
      return {
        icon: <ShieldCheck className="h-4 w-4 text-success" />,
        title: (
          <>
            {entityRef(entry, 'Quote', 'Quote')} signed
            {name ? (
              <>
                {' '}
                by <span className="font-medium">{name}</span>
              </>
            ) : null}
          </>
        ),
        subtitle: ip ? `from ${ip}` : undefined,
      }
    }

    case 'entity.email_sent': {
      const recipients = Array.isArray(md.recipients) ? (md.recipients as string[]) : []
      const recipientCount =
        typeof md.recipient_count === 'number' ? (md.recipient_count as number) : recipients.length
      const subject = typeof md.subject === 'string' ? md.subject : undefined
      const kind = typeof md.kind === 'string' ? md.kind : 'email'
      return {
        icon: <Mail className="h-4 w-4" />,
        title: (
          <>
            Email sent
            {subject ? (
              <>
                : <span className="font-medium">{subject}</span>
              </>
            ) : (
              <> ({kind})</>
            )}
            {recipientCount > 0 && (
              <>
                {', to '}
                {recipients.length > 0 ? (
                  recipientsNode(recipients)
                ) : (
                  <span>
                    {recipientCount} recipient{recipientCount === 1 ? '' : 's'}
                  </span>
                )}
              </>
            )}
          </>
        ),
        emailPreview: { activityId: entry.id },
      }
    }
    case 'entity.email_resent': {
      const kind = typeof md.kind === 'string' ? emailKindName(md.kind) : 'Email'
      return {
        icon: <Send className="h-4 w-4 text-foreground" />,
        title: <>{kind} email resent for {entityRef(entry, entityTypeLabel(entry.entityType), 'this record')}</>,
        subtitle:
          typeof md.delivery_unknown_activity_id === 'string'
            ? 'Sent again after a delivery that could not be confirmed'
            : undefined,
      }
    }
    case 'entity.email_skipped': {
      const kind = typeof md.kind === 'string' ? emailKindName(md.kind) : 'Email'
      const reason = skipReason(md.reason)
      return {
        icon: <MailX className="h-4 w-4 text-muted-foreground" />,
        title: <>{kind} email skipped for {entityRef(entry, entityTypeLabel(entry.entityType), 'this record')}</>,
        subtitle: reason ? `Reason: ${reason}` : undefined,
      }
    }
    case 'entity.email_delivery_unknown': {
      const rawKind = typeof md.kind === 'string' ? md.kind : undefined
      const kind = rawKind ? emailKindName(rawKind) : 'Email'
      const isReminder = rawKind === 'payment_reminder'
      return {
        icon: <MailQuestion className="h-4 w-4 text-warning" />,
        title: <>{kind} email delivery unknown for {entityRef(entry, entityTypeLabel(entry.entityType), 'this record')}</>,
        subtitle: isReminder
          ? 'The send was interrupted before delivery was confirmed and was not retried automatically. If the customer did not receive it, use Send now on the invoice’s reminders card.'
          : 'The send was interrupted before delivery was confirmed and was not retried automatically. If the customer did not receive it, use Send again.',
        sendAgain: rawKind && !isReminder ? { activityId: entry.id, kind } : undefined,
      }
    }

    case 'payment_reminder.enrolled': {
      const via = humanize(md.via)
      return {
        icon: <BellRing className="h-4 w-4 text-success" />,
        title: <>Payment reminders started for {entityRef(entry, 'Invoice', 'Invoice')}</>,
        subtitle: [via ? `via ${via}` : undefined, reminderStepSubtitle(md)]
          .filter(Boolean)
          .join(' · '),
      }
    }
    case 'payment_reminder.not_enrolled': {
      const reason = humanize(md.reason)
      return {
        icon: <BellOff className="h-4 w-4 text-muted-foreground" />,
        title: <>{entityRef(entry, 'Invoice', 'Invoice')} not enrolled in payment reminders</>,
        subtitle: reason ? `Reason: ${reason}` : undefined,
      }
    }
    case 'payment_reminder.step_executed': {
      const offset = typeof md.offset_days === 'number' ? offsetPill(md.offset_days) : undefined
      return {
        icon: <Bell className="h-4 w-4" />,
        title: (
          <>
            Payment reminder{offset ? ` (${offset})` : ''} queued for{' '}
            {entityRef(entry, 'Invoice', 'Invoice')}
          </>
        ),
        subtitle: reminderStepSubtitle(md),
      }
    }
    case 'payment_reminder.skipped': {
      const offset = typeof md.offset_days === 'number' ? offsetPill(md.offset_days) : undefined
      const reason = skipReason(md.reason)
      return {
        icon: <BellMinus className="h-4 w-4 text-muted-foreground" />,
        title: (
          <>
            Payment reminder{offset ? ` (${offset})` : ''} skipped for{' '}
            {entityRef(entry, 'Invoice', 'Invoice')}
          </>
        ),
        subtitle: reason ? `Reason: ${reason}` : undefined,
      }
    }
    case 'payment_reminder.completed':
      return {
        icon: <CheckCircle2 className="h-4 w-4 text-success" />,
        title: <>Payment reminders completed for {entityRef(entry, 'Invoice', 'Invoice')}</>,
      }
    case 'payment_reminder.paused': {
      const until = optionalDateTime(md.until ?? md.paused_until)
      return {
        icon: <CirclePause className="h-4 w-4 text-warning" />,
        title: <>Payment reminders paused for {entityRef(entry, 'Invoice', 'Invoice')}</>,
        subtitle: until ? `until ${until}` : undefined,
      }
    }
    case 'payment_reminder.resumed':
      return {
        icon: <Play className="h-4 w-4 text-success" />,
        title: <>Payment reminders resumed for {entityRef(entry, 'Invoice', 'Invoice')}</>,
      }
    case 'payment_reminder.stopped': {
      const reason = humanize(md.reason ?? md.stop_reason)
      return {
        icon: <OctagonX className="h-4 w-4 text-destructive" />,
        title: <>Payment reminders stopped for {entityRef(entry, 'Invoice', 'Invoice')}</>,
        subtitle: reason ? `Reason: ${reason}` : undefined,
      }
    }
    case 'payment_reminder.restarted':
      return {
        icon: <RotateCcw className="h-4 w-4" />,
        title: <>Payment reminders restarted for {entityRef(entry, 'Invoice', 'Invoice')}</>,
        subtitle: reminderStepSubtitle(md),
      }

    case 'reminder_sequence.created': {
      const name = typeof md.name === 'string' ? md.name : entry.entityName
      const steps = typeof md.steps === 'number' ? md.steps : undefined
      const enabled = typeof md.enabled === 'boolean' ? md.enabled : undefined
      return {
        icon: <Plus className="h-4 w-4 text-success" />,
        title: (
          <>
            <SequenceLink id={entry.entityId} name={name} /> created
          </>
        ),
        subtitle:
          [
            steps !== undefined ? `${steps} step${steps === 1 ? '' : 's'}` : undefined,
            enabled === undefined ? undefined : enabled ? 'enabled' : 'disabled',
          ]
            .filter(Boolean)
            .join(' · ') || undefined,
      }
    }
    case 'reminder_sequence.updated': {
      const changes = Array.isArray(md.changes) ? (md.changes as { field?: string }[]) : []
      const fields = changes.map(c => c.field).filter((x): x is string => typeof x === 'string')
      return {
        icon: <PencilLine className="h-4 w-4" />,
        title: (
          <>
            <SequenceLink id={entry.entityId} name={entry.entityName} /> updated
          </>
        ),
        subtitle: fields.join(', ') || undefined,
      }
    }
    case 'reminder_sequence.archived': {
      const stopped = typeof md.stopped_invoices === 'number' ? md.stopped_invoices : 0
      return {
        icon: <Archive className="h-4 w-4 text-warning" />,
        title: (
          <>
            <SequenceLink id={entry.entityId} name={entry.entityName} /> archived
          </>
        ),
        subtitle:
          stopped > 0
            ? `reminders stopped for ${stopped} invoice${stopped === 1 ? '' : 's'}`
            : undefined,
      }
    }

    case 'entity.field_changed': {
      const field = typeof md.field === 'string' ? md.field : 'field'
      const label = entityTypeLabel(entry.entityType)
      return {
        icon: <PencilLine className="h-4 w-4" />,
        title: (
          <>
            Changed <span className="font-medium">{field}</span> on {entityRef(entry, label, label)}
          </>
        ),
      }
    }
    case 'entity.updated': {
      const changes = Array.isArray(md.changes) ? (md.changes as { field?: string }[]) : []
      const fields = changes.map(c => c.field).filter((x): x is string => typeof x === 'string')
      const label = entityTypeLabel(entry.entityType)
      const ref = entityRef(entry, label, label)
      return {
        icon: <PencilLine className="h-4 w-4" />,
        title:
          fields.length === 0 ? (
            <>Updated {ref}</>
          ) : (
            <>
              Updated {fields.length} field{fields.length === 1 ? '' : 's'} on {ref}
            </>
          ),
        subtitle: fields.join(', ') || undefined,
      }
    }

    case 'api_token.created': {
      const name = typeof md.name === 'string' ? md.name : undefined
      const hint = typeof md.hint === 'string' ? md.hint : undefined
      return {
        icon: <KeyRound className="h-4 w-4 text-success" />,
        title: name ? (
          <>
            API token <span className="font-medium">&quot;{name}&quot;</span> created
          </>
        ) : (
          <>API token created</>
        ),
        subtitle: hint,
      }
    }
    case 'api_token.revoked':
      return {
        icon: <KeyRound className="h-4 w-4 text-destructive" />,
        title: <>API token revoked</>,
      }

    case 'connector.connected': {
      const provider = typeof md.provider === 'string' ? md.provider : 'connector'
      const alias = typeof md.alias === 'string' ? md.alias : undefined
      return {
        icon: <Link2 className="h-4 w-4 text-success" />,
        title: (
          <>
            <span className="font-medium">{provider}</span> connected
          </>
        ),
        subtitle: alias,
      }
    }
    case 'connector.disconnected': {
      const provider = typeof md.provider === 'string' ? md.provider : 'connector'
      const alias = typeof md.alias === 'string' ? md.alias : undefined
      return {
        icon: <Link2Off className="h-4 w-4 text-destructive" />,
        title: (
          <>
            <span className="font-medium">{provider}</span> disconnected
          </>
        ),
        subtitle: alias,
      }
    }

    case 'connector.disabled':
    case 'connector.enabled':
    case 'connector.deleted': {
      const provider = typeof md.provider === 'string' ? md.provider : 'connector'
      const alias = typeof md.alias === 'string' ? md.alias : undefined
      const type = entry.activityType
      const action = type.slice('connector.'.length)
      return {
        icon:
          type === 'connector.enabled' ? (
            <ToggleRight className="h-4 w-4 text-success" />
          ) : type === 'connector.disabled' ? (
            <ToggleLeft className="h-4 w-4 text-warning" />
          ) : (
            <Trash2 className="h-4 w-4 text-destructive" />
          ),
        title: (
          <>
            <span className="font-medium">{provider}</span> connection {action}
          </>
        ),
        subtitle: alias,
      }
    }
    case 'connector.credentials_updated': {
      const provider = typeof md.provider === 'string' ? md.provider : 'connector'
      const alias = typeof md.alias === 'string' ? md.alias : undefined
      const from = typeof md.from_key_hint === 'string' ? md.from_key_hint : undefined
      const to = typeof md.to_key_hint === 'string' ? md.to_key_hint : undefined
      const keys = from && to && from !== to ? `${from} → ${to}` : to
      return {
        icon: <KeyRound className="h-4 w-4" />,
        title: (
          <>
            <span className="font-medium">{provider}</span> credentials updated
          </>
        ),
        subtitle: [alias, keys].filter(Boolean).join(' · ') || undefined,
      }
    }

    case 'subscription.paused':
      return {
        icon: <CalendarX className="h-4 w-4 text-warning" />,
        title: <>{entityRef(entry, 'Subscription on', 'Subscription')} paused</>,
      }
    case 'subscription.cancellation_scheduled': {
      const reason = typeof md.reason === 'string' ? md.reason : undefined
      return {
        icon: <CalendarX className="h-4 w-4 text-warning" />,
        title: <>{entityRef(entry, 'Subscription on', 'Subscription')} cancellation scheduled</>,
        subtitle: reason,
      }
    }
    case 'subscription.cancelled':
      return {
        icon: <XCircle className="h-4 w-4 text-destructive" />,
        title: <>{entityRef(entry, 'Subscription on', 'Subscription')} cancelled</>,
      }
    case 'subscription.cancellation_undone':
      return {
        icon: <CheckCircle2 className="h-4 w-4 text-success" />,
        title: <>{entityRef(entry, 'Subscription on', 'Subscription')} cancellation undone</>,
      }
    case 'subscription.plan_change_scheduled': {
      const effectiveAt = typeof md.effective_at === 'string' ? md.effective_at : undefined
      return {
        icon: <Send className="h-4 w-4 text-foreground" />,
        title: <>{entityRef(entry, 'Subscription on', 'Subscription')} plan change scheduled</>,
        subtitle: effectiveAt ? `effective ${effectiveAt}` : undefined,
      }
    }
    case 'subscription.plan_change_cancelled':
      return {
        icon: <XCircle className="h-4 w-4 text-warning" />,
        title: <>{entityRef(entry, 'Subscription on', 'Subscription')} plan change cancelled</>,
      }
    case 'subscription.plan_changed': {
      const changeDate = typeof md.change_date === 'string' ? md.change_date : undefined
      return {
        icon: <CheckCircle2 className="h-4 w-4 text-success" />,
        title: <>{entityRef(entry, 'Subscription on', 'Subscription')} plan changed</>,
        subtitle: changeDate ? `effective ${changeDate}` : undefined,
      }
    }
    case 'subscription.amendment_scheduled': {
      const effectiveAt = typeof md.effective_at === 'string' ? md.effective_at : undefined
      return {
        icon: <Send className="h-4 w-4 text-foreground" />,
        title: <>{entityRef(entry, 'Subscription on', 'Subscription')} amendment scheduled</>,
        subtitle: effectiveAt ? `effective ${effectiveAt}` : undefined,
      }
    }
    case 'subscription.amendment_cancelled':
      return {
        icon: <XCircle className="h-4 w-4 text-warning" />,
        title: <>{entityRef(entry, 'Subscription on', 'Subscription')} amendment cancelled</>,
      }
    case 'subscription.amended': {
      const effectiveAt = typeof md.effective_at === 'string' ? md.effective_at : undefined
      return {
        icon: <CheckCircle2 className="h-4 w-4 text-success" />,
        title: <>{entityRef(entry, 'Subscription on', 'Subscription')} amended</>,
        subtitle: effectiveAt ? `effective ${effectiveAt}` : undefined,
      }
    }
    case 'subscription.slots_changed': {
      const from = typeof md.from === 'number' ? md.from : undefined
      const to = typeof md.to === 'number' ? md.to : undefined
      const delta = typeof md.delta === 'number' ? md.delta : undefined
      const applied = md.applied === true
      return {
        icon: <Users className="h-4 w-4" />,
        title: (
          <>
            {entityRef(entry, 'Subscription on', 'Subscription')} seats{' '}
            {from !== undefined && to !== undefined ? (
              <>
                changed {from} → {to}
              </>
            ) : (
              'changed'
            )}
          </>
        ),
        subtitle:
          !applied && delta !== undefined ? `pending ${delta > 0 ? '+' : ''}${delta}` : undefined,
      }
    }

    default:
      return {
        icon: <Activity className="h-4 w-4 text-muted-foreground" />,
        title: <>{entry.activityType}</>,
      }
  }
}

const SendAgainButton = ({ activityId, kind }: { activityId: string; kind: string }) => {
  const showConfirmModal = useConfirmationModal()
  const queryClient = useQueryClient()
  // One id per mounted entry: double clicks and retries collapse into a single send.
  const [requestId] = useState(() => crypto.randomUUID())
  const [queued, setQueued] = useState(false)
  const sendAgainMut = useMutation(resendUnknownEmail, {
    onSuccess: () => {
      setQueued(true)
      toast.success('Queued')
      queryClient.invalidateQueries({
        queryKey: createConnectQueryKey({ schema: listEntityActivity, cardinality: undefined }),
      })
      queryClient.invalidateQueries({
        queryKey: createConnectQueryKey({ schema: listActivity, cardinality: undefined }),
      })
      queryClient.invalidateQueries({
        queryKey: createConnectQueryKey({ schema: listSentEmails, cardinality: undefined }),
      })
    },
    onError: err => toast.error(err.rawMessage || 'Could not send this email again'),
  })

  const sendAgain = () =>
    showConfirmModal(
      () => sendAgainMut.mutate({ activityId, requestId }),
      {
        header: 'Send this email again?',
        message: `The ${kind.toLowerCase()} email will be rendered again from the current data and sent to the customer’s current recipients. If the first send did arrive, they will receive it twice.`,
      },
      { buttonLabel: 'Send again', danger: false }
    )

  return (
    <Button
      variant="ghost"
      size="sm"
      className="ml-2 h-5 px-2 text-[11px] align-baseline"
      disabled={queued || sendAgainMut.isPending}
      onClick={sendAgain}
    >
      {queued ? 'Queued' : 'Send again'}
    </Button>
  )
}

export const ActivityEntryRow = ({ entry, compact = false }: Props) => {
  const rendered = renderEntry(entry)
  const occurred = new Date(entry.occurredAt)
  const [previewOpen, setPreviewOpen] = useState(false)

  const actor = actorByline(entry)
  const metaNode = (
    <div className="flex items-baseline gap-1.5 text-[11px] text-muted-foreground whitespace-nowrap">
      {actor}
      {actor && <span aria-hidden>·</span>}
      <time dateTime={entry.occurredAt} title={occurred.toLocaleString()}>
        {formatDistanceToNow(occurred, { addSuffix: true })}
      </time>
    </div>
  )

  const titleRowClass = compact
    ? 'flex flex-col gap-0.5'
    : 'flex flex-col gap-0.5 sm:flex-row sm:items-baseline sm:justify-between sm:gap-3'

  return (
    <>
      <li className="flex items-start gap-2.5 py-2">
        <div className="flex h-[18px] shrink-0 items-center">
          <div className="flex h-6 w-6 items-center justify-center rounded-full border bg-muted/40">
            {rendered.icon}
          </div>
        </div>
        <div className="flex-1 min-w-0">
          <div className={titleRowClass}>
            <div className="text-[13px] leading-snug break-words min-w-0">
              {rendered.title}
              {rendered.emailPreview && (
                <Button
                  variant="ghost"
                  size="sm"
                  className="ml-2 h-5 px-2 text-[11px] align-baseline"
                  onClick={() => setPreviewOpen(true)}
                >
                  Preview
                </Button>
              )}
              {rendered.sendAgain && (
                <Gate perm="billing:invoices:manage">
                  <SendAgainButton
                    activityId={rendered.sendAgain.activityId}
                    kind={rendered.sendAgain.kind}
                  />
                </Gate>
              )}
            </div>
            <div className="shrink-0">{metaNode}</div>
          </div>
          {rendered.subtitle && (
            <div className="text-[11px] text-muted-foreground break-words mt-0.5">
              {rendered.subtitle}
            </div>
          )}
        </div>
      </li>
      {rendered.emailPreview && previewOpen && (
        <EmailPreviewDialog
          sentEmailId={rendered.emailPreview.activityId}
          open={previewOpen}
          onOpenChange={setPreviewOpen}
        />
      )}
    </>
  )
}
