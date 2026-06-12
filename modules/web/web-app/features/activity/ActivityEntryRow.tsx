import { Code } from '@connectrpc/connect'
import {
  Button,
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  HoverCard,
  HoverCardContent,
  HoverCardTrigger,
  Skeleton,
} from '@md/ui'
import { formatDistanceToNow } from 'date-fns'
import {
  Activity,
  Archive,
  CalendarX,
  CheckCircle2,
  CircleX,
  Download,
  Eye,
  FileText,
  KeyRound,
  Link2,
  Link2Off,
  LogIn,
  Mail,
  Paperclip,
  PencilLine,
  Plus,
  Receipt,
  Send,
  ShieldCheck,
  XCircle,
} from 'lucide-react'
import { ReactNode, useState } from 'react'

import { useQuery } from '@/lib/connectrpc'
import { env } from '@/lib/env'
import { getSentEmail } from '@/rpc/api/activity/v1/activity-ActivityService_connectquery'
import { ActivityEntry, ActorType } from '@/rpc/api/activity/v1/activity_pb'

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
    default:
      return 'Unknown'
  }
}

function entityRef(entry: ActivityEntry, prefix: string, fallback: string): ReactNode {
  const label = entry.entityName ? `${prefix} ${entry.entityName}` : fallback
  return (
    <EntityHoverCard
      entityType={entry.entityType}
      entityId={entry.entityId}
      label={label}
    />
  )
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
          {recipients.map((r) => (
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
            {entityRef(entry, 'Quote', 'Quote')} signed{name ? <> by <span className="font-medium">{name}</span></> : null}
          </>
        ),
        subtitle: ip ? `from ${ip}` : undefined,
      }
    }

    case 'entity.email_sent': {
      const recipients = Array.isArray(md.recipients) ? (md.recipients as string[]) : []
      const recipientCount = typeof md.recipient_count === 'number'
        ? (md.recipient_count as number)
        : recipients.length
      const subject = typeof md.subject === 'string' ? md.subject : undefined
      const kind = typeof md.kind === 'string' ? md.kind : 'email'
      return {
        icon: <Mail className="h-4 w-4" />,
        title: (
          <>
            Email sent{subject ? <>: <span className="font-medium">{subject}</span></> : <> ({kind})</>}
            {recipientCount > 0 && (
              <>
                {', to '}
                {recipients.length > 0
                  ? recipientsNode(recipients)
                  : <span>{recipientCount} recipient{recipientCount === 1 ? '' : 's'}</span>}
              </>
            )}
          </>
        ),
        emailPreview: { activityId: entry.id },
      }
    }

    case 'entity.field_changed': {
      const field = typeof md.field === 'string' ? md.field : 'field'
      return {
        icon: <PencilLine className="h-4 w-4" />,
        title: <>Changed <span className="font-medium">{field}</span></>,
      }
    }
    case 'entity.updated': {
      const changes = Array.isArray(md.changes) ? (md.changes as { field?: string }[]) : []
      const fields = changes
        .map((c) => c.field)
        .filter((x): x is string => typeof x === 'string')
      return {
        icon: <PencilLine className="h-4 w-4" />,
        title: <>Updated {fields.length} field{fields.length === 1 ? '' : 's'}</>,
        subtitle: fields.join(', ') || undefined,
      }
    }

    case 'api_token.created': {
      const name = typeof md.name === 'string' ? md.name : undefined
      const hint = typeof md.hint === 'string' ? md.hint : undefined
      return {
        icon: <KeyRound className="h-4 w-4 text-success" />,
        title: name
          ? <>API token <span className="font-medium">&quot;{name}&quot;</span> created</>
          : <>API token created</>,
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
        title: <><span className="font-medium">{provider}</span> connected</>,
        subtitle: alias,
      }
    }
    case 'connector.disconnected': {
      const provider = typeof md.provider === 'string' ? md.provider : 'connector'
      const alias = typeof md.alias === 'string' ? md.alias : undefined
      return {
        icon: <Link2Off className="h-4 w-4 text-destructive" />,
        title: <><span className="font-medium">{provider}</span> disconnected</>,
        subtitle: alias,
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

    default:
      return {
        icon: <Activity className="h-4 w-4 text-muted-foreground" />,
        title: <>{entry.activityType}</>,
      }
  }
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
        <div className="flex h-6 w-6 shrink-0 items-center justify-center rounded-full border bg-muted/40 mt-0.5">
          {rendered.icon}
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
          activityId={rendered.emailPreview.activityId}
          open={previewOpen}
          onOpenChange={setPreviewOpen}
        />
      )}
    </>
  )
}

const EmailPreviewDialog = ({
  activityId,
  open,
  onOpenChange,
}: {
  activityId: string
  open: boolean
  onOpenChange: (o: boolean) => void
}) => {
  const query = useQuery(
    getSentEmail,
    { activityId },
    {
      enabled: open,
      staleTime: Infinity,
      // A purged/missing preview is a deterministic NotFound — don't retry it.
      retry: (count, err) => err.code !== Code.NotFound && count < 2,
    }
  )
  const email = query.data?.email
  const isGone = query.error?.code === Code.NotFound
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-3xl">
        <DialogHeader>
          <DialogTitle>{email?.subject ?? 'Email preview'}</DialogTitle>
        </DialogHeader>
        {query.isLoading && (
          <div className="space-y-2 py-4">
            <Skeleton height={16} width={240} />
            <Skeleton height={200} width="100%" />
          </div>
        )}
        {query.isError && (
          <div className="py-6 text-sm text-muted-foreground">
            {isGone
              ? 'This email preview is no longer available. Previews are retained for a limited time — the delivery record remains in the activity log.'
              : 'Couldn’t load this email preview. Please try again.'}
          </div>
        )}
        {email && (
          <div className="space-y-3">
            <div className="text-xs text-muted-foreground space-y-0.5">
              <div>
                <span className="font-medium">From:</span> {email.fromAddr}
              </div>
              <div>
                <span className="font-medium">To:</span> {email.recipients.join(', ')}
              </div>
              {email.replyTo && (
                <div>
                  <span className="font-medium">Reply-To:</span> {email.replyTo}
                </div>
              )}
            </div>
            {email.attachments.length > 0 && (
              <div className="flex flex-wrap gap-2">
                {email.attachments.map((att, i) => {
                  const downloadable = att.id && email.attachmentsShareKey
                  const url = downloadable
                    ? `${env.meteroidRestApiUri}/files/v1/email/attachment/${activityId}/${att.id}?token=${email.attachmentsShareKey}`
                    : undefined
                  const className =
                    'inline-flex items-center gap-1.5 rounded border px-2 py-1 text-xs text-muted-foreground'
                  return url ? (
                    <a
                      key={i}
                      href={url}
                      target="_blank"
                      rel="noopener noreferrer"
                      className={`${className} hover:bg-accent hover:text-accent-foreground`}
                    >
                      <Download className="h-3.5 w-3.5" />
                      {att.filename}
                    </a>
                  ) : (
                    <span key={i} className={className}>
                      <Paperclip className="h-3.5 w-3.5" />
                      {att.filename}
                    </span>
                  )
                })}
              </div>
            )}
            <iframe
              title="email preview"
              srcDoc={email.bodyHtml}
              sandbox=""
              className="w-full h-[60vh] border rounded bg-background"
            />
          </div>
        )}
      </DialogContent>
    </Dialog>
  )
}
