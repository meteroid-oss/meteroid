import { Code } from '@connectrpc/connect'
import { createConnectQueryKey, skipToken, useMutation } from '@connectrpc/connect-query'
import { Badge, Button, cn, Switch } from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { CalendarClock, Users } from 'lucide-react'
import { ReactNode, useMemo, useState } from 'react'
import { toast } from 'sonner'

import { slotItems } from '@/edition'
import { SettingsRowGroup } from '@/features/settings/components/SettingsSection'
import { useInvoicingEntity } from '@/features/settings/hooks/useInvoicingEntity'
import { usePermissions } from '@/hooks/usePermissions'
import { useQuery } from '@/lib/connectrpc'
import {
  getEmailSettings,
  setEmailKindEnabled,
} from '@/rpc/api/customeremails/v1/customeremails-CustomerEmailsService_connectquery'
import { GetEmailSettingsResponse } from '@/rpc/api/customeremails/v1/customeremails_pb'
import { EmailKind, EmailKindInfo } from '@/rpc/api/customeremails/v1/models_pb'

import { EmailKindAudienceSheet, sendToSummary } from './EmailKindAudienceSheet'
import { EmailPreviewPanel } from './EmailPreviewPanel'
import { parseAudience } from './audience/model'
import { useAudienceNames } from './audience/useAudienceNames'

interface EmailKindMeta {
  kind: EmailKind
  label: string
  description: string
  editableCopy: boolean
  attachesDocument: boolean
  /** Sent on the schedule a module's view edits: listed only when one does. */
  scheduled?: boolean
  /** Sent by a module only: listed only when one declares it. */
  sentByModule?: boolean
}

interface KindGroup {
  title: string
  kinds: EmailKindMeta[]
  placeholders?: { label: string; description: string }[]
}

// Static catalogue: labels and chip defaults when the server has not answered yet.
// `EmailKindInfo` from the server wins for the enabled/editable/attachment flags.
export const EMAIL_KIND_META: EmailKindMeta[] = [
  {
    kind: EmailKind.INVOICE_READY,
    label: 'Invoice ready',
    description: 'Sent when an invoice is finalized.',
    editableCopy: false,
    attachesDocument: true,
  },
  {
    kind: EmailKind.INVOICE_PAID,
    label: 'Receipt',
    description: 'Sent when an invoice is paid in full.',
    editableCopy: false,
    attachesDocument: true,
  },
  {
    kind: EmailKind.CREDIT_NOTE_ISSUED,
    label: 'Credit note issued',
    description: 'Sent when a credit note is finalized.',
    editableCopy: false,
    attachesDocument: true,
  },
  {
    kind: EmailKind.PAYMENT_FAILED,
    label: 'Payment failed',
    description: 'Sent when an automatic charge fails and the invoice needs attention.',
    editableCopy: false,
    attachesDocument: true,
  },
  {
    kind: EmailKind.AUTHENTICATION_REQUIRED,
    label: 'Authentication required',
    description: 'Sent when the bank asks the customer to confirm a charge (3-D Secure).',
    editableCopy: false,
    attachesDocument: false,
  },
  {
    kind: EmailKind.PAYMENT_REMINDER,
    label: 'Payment reminders',
    description: 'Scheduled before and after the due date for invoices awaiting payment.',
    editableCopy: true,
    attachesDocument: true,
    scheduled: true,
  },
  {
    kind: EmailKind.QUOTE_READY,
    label: 'Quote ready',
    description: 'Sent to the quote recipients when you click Send to customer.',
    editableCopy: false,
    attachesDocument: false,
  },
  {
    kind: EmailKind.WALLET_BALANCE_LOW,
    label: 'Low balance',
    description: "Sent when a customer's prepaid balance falls below its alert threshold.",
    editableCopy: false,
    attachesDocument: false,
    sentByModule: true,
  },
  {
    kind: EmailKind.AUTO_TOP_UP_FAILED,
    label: 'Automatic top-up failed',
    description: 'Sent when an automatic top-up cannot be charged, and when top-ups are paused.',
    editableCopy: false,
    attachesDocument: false,
    sentByModule: true,
  },
  {
    kind: EmailKind.BUDGET_THRESHOLD,
    label: 'Budget threshold reached',
    description: "Sent when a customer's spend reaches a notification level of its budget.",
    editableCopy: false,
    attachesDocument: false,
    sentByModule: true,
  },
  {
    kind: EmailKind.BUDGET_EXCEEDED,
    label: 'Budget reached',
    description: "Sent when a customer's spend reaches its budget.",
    editableCopy: false,
    attachesDocument: false,
    sentByModule: true,
  },
]

const byKind = (kind: EmailKind) => EMAIL_KIND_META.find(m => m.kind === kind)

const GROUPS: KindGroup[] = [
  {
    title: 'Invoices & credit notes',
    kinds: [EmailKind.INVOICE_READY, EmailKind.INVOICE_PAID, EmailKind.CREDIT_NOTE_ISSUED].map(k =>
      byKind(k)!
    ),
  },
  {
    title: 'Payments',
    kinds: [EmailKind.PAYMENT_FAILED, EmailKind.AUTHENTICATION_REQUIRED].map(k => byKind(k)!),
    placeholders: [
      { label: 'Checkout link', description: 'A link to pay an invoice by card or direct debit.' },
      { label: 'Portal sign-in', description: 'A sign-in link to the customer portal.' },
    ],
  },
  { title: 'Collections', kinds: [byKind(EmailKind.PAYMENT_REMINDER)!] },
  { title: 'Quotes', kinds: [byKind(EmailKind.QUOTE_READY)!] },
  {
    title: 'Prepaid balance',
    kinds: [EmailKind.WALLET_BALANCE_LOW, EmailKind.AUTO_TOP_UP_FAILED].map(k => byKind(k)!),
  },
  {
    title: 'Budgets',
    kinds: [EmailKind.BUDGET_THRESHOLD, EmailKind.BUDGET_EXCEEDED].map(k => byKind(k)!),
  },
]

export const emailKindLabel = (kind: EmailKind): string => byKind(kind)?.label ?? 'Email'

export const emailSettingsQueryKey = (invoicingEntityId: string) =>
  createConnectQueryKey({
    schema: getEmailSettings,
    input: { invoicingEntityId },
    cardinality: 'finite',
  })

const KindRow = ({
  title,
  description,
  chips,
  footer,
  selected,
  onSelect,
  children,
}: {
  title: string
  description: string
  chips?: ReactNode
  footer?: ReactNode
  selected?: boolean
  onSelect?: () => void
  children: ReactNode
}) => (
  <div
    role={onSelect ? 'button' : undefined}
    tabIndex={onSelect ? 0 : undefined}
    aria-pressed={onSelect ? selected : undefined}
    onClick={onSelect}
    onKeyDown={e => {
      if (onSelect && (e.key === 'Enter' || e.key === ' ')) {
        e.preventDefault()
        onSelect()
      }
    }}
    className={cn(
      'flex items-center gap-4 border-t border-border px-4 py-3 first:border-t-0 transition-colors',
      onSelect && 'cursor-pointer hover:bg-accent/40',
      selected && 'bg-accent/60'
    )}
  >
    <div className="min-w-0 space-y-0.5">
      <div className="flex flex-wrap items-center gap-2 text-[13px] font-medium">
        {title}
        {chips}
      </div>
      <div className="text-xs leading-snug text-muted-foreground">{description}</div>
      {footer}
    </div>
    <div
      className="ml-auto flex shrink-0 items-center gap-2.5"
      onClick={e => e.stopPropagation()}
      onKeyDown={e => e.stopPropagation()}
    >
      {children}
    </div>
  </div>
)

const SendToButton = ({ summary, onClick }: { summary: string; onClick: () => void }) => (
  <button
    type="button"
    title={summary}
    onClick={e => {
      e.stopPropagation()
      onClick()
    }}
    onKeyDown={e => e.stopPropagation()}
    className="flex max-w-full items-center gap-1.5 pt-1 text-xs text-muted-foreground hover:text-foreground"
  >
    <Users size={12} className="shrink-0" />
    <span className="shrink-0">Send to:</span>
    <span className="truncate font-medium text-foreground">{summary}</span>
  </button>
)

const KindChips = ({
  info,
}: {
  info: Pick<EmailKindInfo, 'editableCopy' | 'attachesDocument'>
}) => (
  <>
    {info.editableCopy && (
      <Badge variant="outline" size="sm" className="font-normal">
        Editable copy
      </Badge>
    )}
    {info.attachesDocument && (
      <Badge variant="outline" size="sm" className="font-normal">
        PDF attached
      </Badge>
    )}
  </>
)

const scheduleViewOf = (kind: EmailKind) =>
  slotItems('customerEmails.views').find(view => view.schedules === kind)

const listedGroups = () => {
  const moduleKinds = slotItems('customerEmails.kinds')
  return GROUPS.map(group => ({
    ...group,
    kinds: group.kinds.filter(
      meta =>
        (!meta.scheduled || scheduleViewOf(meta.kind)) &&
        (!meta.sentByModule || moduleKinds.includes(meta.kind))
    ),
  })).filter(group => group.kinds.length > 0 || (group.placeholders?.length ?? 0) > 0)
}

export const EmailKindsView = ({ onOpenView }: { onOpenView: (view: string) => void }) => {
  const queryClient = useQueryClient()
  const groups = listedGroups()
  const { selectedEntityId: entityId, currentEntity } = useInvoicingEntity()
  const canManage = usePermissions().has('settings:manage')
  const [selectedKind, setSelectedKind] = useState<EmailKind>(EmailKind.INVOICE_READY)
  const [audienceKind, setAudienceKind] = useState<EmailKind | null>(null)
  const [audienceOpen, setAudienceOpen] = useState(false)

  const settingsQuery = useQuery(
    getEmailSettings,
    entityId ? { invoicingEntityId: entityId } : skipToken,
    { retry: (count, err) => err.code !== Code.Unimplemented && count < 2 }
  )
  const settings = settingsQuery.data?.settings
  const unavailable = settingsQuery.error?.code === Code.Unimplemented
  const kindInfo = (kind: EmailKind) => settings?.kinds.find(k => k.kind === kind)

  const rulePredicates = useMemo(
    () => (settings?.kinds ?? []).flatMap(k => parseAudience(k.audience?.audience)),
    [settings?.kinds]
  )
  const { ctx: audienceNames } = useAudienceNames(rulePredicates)
  const audienceInfo = audienceKind !== null ? kindInfo(audienceKind) : undefined

  const queryKey = entityId ? emailSettingsQueryKey(entityId) : undefined
  const toggleMut = useMutation(setEmailKindEnabled, {
    onMutate: async vars => {
      if (!queryKey) return { previous: [] }
      await queryClient.cancelQueries({ queryKey })
      const previous = queryClient.getQueriesData<GetEmailSettingsResponse>({ queryKey })
      queryClient.setQueriesData<GetEmailSettingsResponse>({ queryKey }, old =>
        old?.settings
          ? {
              ...old,
              settings: {
                ...old.settings,
                kinds: old.settings.kinds.map(k =>
                  k.kind === vars.kind ? { ...k, enabled: !!vars.enabled } : k
                ),
              },
            }
          : old
      )
      return { previous }
    },
    onError: (err, _vars, ctx) => {
      ctx?.previous.forEach(([key, data]) => queryClient.setQueryData(key, data))
      toast.error(err.rawMessage || 'Could not update this email')
    },
    onSettled: () => queryKey && queryClient.invalidateQueries({ queryKey }),
  })

  const toggle = (kind: EmailKind, enabled: boolean) => {
    if (!entityId) return
    toggleMut.mutate({ invoicingEntityId: entityId, kind, enabled })
  }

  const switchesDisabled = !canManage || !settings

  return (
    <div className="grid gap-6 lg:grid-cols-[minmax(0,5fr)_minmax(0,6fr)]">
      <div className="space-y-5">
        {unavailable && (
          <p className="text-[13px] text-muted-foreground">
            Email settings are not available on this server yet — toggles are read-only.
          </p>
        )}
        {groups.map(group => (
          <div key={group.title} className="space-y-2">
            <div className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
              {group.title}
            </div>
            <SettingsRowGroup>
              {group.kinds.map(meta => {
                const serverInfo = kindInfo(meta.kind)
                const info = serverInfo ?? meta
                const scheduleView = scheduleViewOf(meta.kind)
                return (
                  <KindRow
                    key={meta.kind}
                    title={meta.label}
                    description={meta.description}
                    chips={<KindChips info={info} />}
                    footer={
                      serverInfo &&
                      !meta.scheduled && (
                        <SendToButton
                          summary={sendToSummary(serverInfo, audienceNames)}
                          onClick={() => {
                            setAudienceKind(meta.kind)
                            setAudienceOpen(true)
                          }}
                        />
                      )
                    }
                    selected={selectedKind === meta.kind}
                    onSelect={() => setSelectedKind(meta.kind)}
                  >
                    {scheduleView && (
                      <Button
                        type="button"
                        variant="ghost"
                        size="sm"
                        hasIcon
                        className="h-7 text-[13px]"
                        onClick={() => onOpenView(scheduleView.value)}
                      >
                        <CalendarClock size={13} />
                        Edit schedule
                      </Button>
                    )}
                    <Switch
                      checked={kindInfo(meta.kind)?.enabled ?? false}
                      disabled={switchesDisabled}
                      onCheckedChange={v => toggle(meta.kind, v)}
                      aria-label={`${meta.label} emails`}
                    />
                  </KindRow>
                )
              })}
              {group.placeholders?.map(p => (
                <KindRow
                  key={p.label}
                  title={p.label}
                  description={p.description}
                  chips={
                    <Badge variant="ghost" size="sm" className="font-normal">
                      Later
                    </Badge>
                  }
                >
                  <Switch checked={false} disabled />
                </KindRow>
              ))}
            </SettingsRowGroup>
          </div>
        ))}
      </div>
      <div className="lg:sticky lg:top-0 lg:self-start">
        {entityId && <EmailPreviewPanel entityId={entityId} kind={selectedKind} />}
      </div>
      {entityId && queryKey && audienceInfo && (
        <EmailKindAudienceSheet
          open={audienceOpen}
          onOpenChange={setAudienceOpen}
          entityId={entityId}
          kindLabel={emailKindLabel(audienceInfo.kind)}
          info={audienceInfo}
          settingsQueryKey={queryKey}
          defaultCurrency={currentEntity?.accountingCurrency}
        />
      )}
    </div>
  )
}
