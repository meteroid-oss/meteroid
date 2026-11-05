import { Code } from '@connectrpc/connect'
import { createConnectQueryKey, skipToken, useMutation } from '@connectrpc/connect-query'
import { Badge, Button, cn, Skeleton, Switch } from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { AlertTriangle, ArrowDown, ArrowUp, Archive, Pencil, PlusIcon } from 'lucide-react'
import { useMemo, useState } from 'react'
import { toast } from 'sonner'

import { emailSettingsQueryKey } from '@/features/customer-emails/EmailKindsView'
import { audienceSummary, parseAudience } from '@/features/customer-emails/audience/model'
import { useAudienceNames } from '@/features/customer-emails/audience/useAudienceNames'
import { enabledLocales } from '@/features/customer-emails/locales'
import { useInvoicingEntity } from '@/features/settings/hooks/useInvoicingEntity'
import { usePermissions } from '@/hooks/usePermissions'
import { useQueryState } from '@/hooks/useQueryState'
import { useQuery } from '@/lib/connectrpc'
import {
  getEmailSettings,
  setEmailKindEnabled,
} from '@/rpc/api/customeremails/v1/customeremails-CustomerEmailsService_connectquery'
import { EmailKind } from '@/rpc/api/customeremails/v1/models_pb'
import { PaymentReminderSequence } from '@/rpc/api/paymentreminders/v1/models_pb'
import {
  archivePaymentReminderSequence,
  listPaymentReminderSequences,
  reorderPaymentReminderSequences,
  updatePaymentReminderSequence,
} from '@/rpc/api/paymentreminders/v1/paymentreminders-PaymentRemindersService_connectquery'
import { useConfirmationModal } from 'providers/ConfirmationProvider'

import { ReminderSchedule } from './ReminderSchedule'
import { DISABLED_HELP, SequenceSheet } from './SequenceSheet'
import { localeListLabel, sequenceMissingLocales } from './stepCopy'

const DEFAULT_TITLE = 'Everyone else'

const stepsSummary = (sequence: PaymentReminderSequence) => {
  if (sequence.steps.length === 0) return 'No steps'
  const offsets = sequence.steps.map(s => s.offsetDays)
  const plural = sequence.steps.length > 1 ? 's' : ''
  const signed = (n: number) => (n < 0 ? `−${-n}` : n > 0 ? `+${n}` : '0')
  return `${sequence.steps.length} step${plural} · day ${signed(Math.min(...offsets))} → ${signed(Math.max(...offsets))}`
}

const inFlightLabel = (n: number) => `${n} invoice${n === 1 ? '' : 's'} in progress`

const isUnimplemented = (err?: { code: Code } | null) => err?.code === Code.Unimplemented

export const RemindersView = () => {
  const queryClient = useQueryClient()
  const showConfirmModal = useConfirmationModal()
  const { selectedEntityId: entityId, currentEntity } = useInvoicingEntity()
  const canManage = usePermissions().has('settings:manage')
  const [sequenceParam, setSequenceParam] = useQueryState<string>('sequence', '', { replace: true })
  const [creating, setCreating] = useState(false)

  const retry = (count: number, err: { code: Code }) => !isUnimplemented(err) && count < 2

  const settingsQuery = useQuery(
    getEmailSettings,
    entityId ? { invoicingEntityId: entityId } : skipToken,
    { retry }
  )
  const settings = settingsQuery.data?.settings
  const kindEnabled =
    settings?.kinds.find(k => k.kind === EmailKind.PAYMENT_REMINDER)?.enabled ?? false
  const homeLocale =
    settings?.effectiveLocale || currentEntity?.effectiveDocumentLanguage || 'en-US'
  const locales = useMemo(() => enabledLocales(settings, homeLocale), [settings, homeLocale])

  const listQuery = useQuery(
    listPaymentReminderSequences,
    entityId ? { invoicingEntityId: entityId, includeArchived: false } : skipToken,
    { retry }
  )
  // Settings gate only the master switch and the send window; the sequences stand on their own.
  const settingsUnavailable = isUnimplemented(settingsQuery.error)
  const unavailable = isUnimplemented(listQuery.error)

  // The default is pinned last whatever its stored position.
  const sequences = useMemo(() => {
    const all = [...(listQuery.data?.sequences ?? [])].sort((a, b) => a.position - b.position)
    return [...all.filter(s => !s.isDefault), ...all.filter(s => s.isDefault)]
  }, [listQuery.data?.sequences])
  const ordered = sequences.filter(s => !s.isDefault)
  const editing = sequenceParam ? (sequences.find(s => s.id === sequenceParam) ?? null) : null

  const audiencePredicates = useMemo(
    () => sequences.flatMap(s => (s.isDefault ? [] : parseAudience(s.audience))),
    [sequences]
  )
  const { ctx: audienceNames } = useAudienceNames(audiencePredicates)

  const invalidateList = () =>
    queryClient.invalidateQueries({
      queryKey: createConnectQueryKey({
        schema: listPaymentReminderSequences,
        cardinality: undefined,
      }),
    })
  const mutationError = (fallback: string) => (err: { code: Code; rawMessage: string }) =>
    toast.error(
      isUnimplemented(err)
        ? 'Payment reminders are not available on this server yet.'
        : err.rawMessage || fallback
    )

  const kindMut = useMutation(setEmailKindEnabled, {
    onSuccess: () =>
      entityId && queryClient.invalidateQueries({ queryKey: emailSettingsQueryKey(entityId) }),
    onError: mutationError('Could not update payment reminders'),
  })
  const updateMut = useMutation(updatePaymentReminderSequence, {
    onSuccess: invalidateList,
    onError: mutationError('Could not update the sequence'),
  })
  const reorderMut = useMutation(reorderPaymentReminderSequences, {
    onSuccess: invalidateList,
    onError: mutationError('Could not reorder the sequences'),
  })
  const archiveMut = useMutation(archivePaymentReminderSequence, {
    onSuccess: res => {
      invalidateList()
      toast.success(
        res.stoppedInvoices > 0
          ? `Sequence archived · reminders stopped for ${res.stoppedInvoices} invoice${res.stoppedInvoices === 1 ? '' : 's'}`
          : 'Sequence archived'
      )
    },
    onError: mutationError('Could not archive the sequence'),
  })

  const toggleSequence = (sequence: PaymentReminderSequence, enabled: boolean) => {
    const run = () => updateMut.mutate({ id: sequence.id, enabled })
    if (!enabled && sequence.inFlightInvoices > 0) {
      showConfirmModal(
        run,
        {
          header: `Disable ${sequence.isDefault ? DEFAULT_TITLE : sequence.name}`,
          message: `${inFlightLabel(sequence.inFlightInvoices)} will be held until this sequence is enabled again. ${DISABLED_HELP}`,
        },
        { buttonLabel: 'Disable' }
      )
      return
    }
    run()
  }

  const move = (index: number, delta: number) => {
    if (!entityId) return
    const target = index + delta
    if (target < 0 || target >= ordered.length) return
    const ids = ordered.map(s => s.id)
    ;[ids[index], ids[target]] = [ids[target], ids[index]]
    reorderMut.mutate({ invoicingEntityId: entityId, ids })
  }

  const archive = (sequence: PaymentReminderSequence) =>
    showConfirmModal(
      () => archiveMut.mutate({ id: sequence.id }),
      {
        header: `Archive ${sequence.name}`,
        message:
          sequence.inFlightInvoices > 0
            ? `Reminders stop for the ${inFlightLabel(sequence.inFlightInvoices)}; they are not re-enrolled elsewhere. Customers pointed at this sequence go back to the default rules.`
            : 'No invoice is in progress. Customers pointed at this sequence go back to the default rules.',
      },
      { buttonLabel: 'Archive' }
    )

  const controlsDisabled = !canManage || unavailable || !entityId

  return (
    <div className="flex flex-col gap-5">
      <div className="flex flex-wrap items-start justify-between gap-4">
        <div className="space-y-1">
          <div className="text-sm font-medium">Payment reminders</div>
          <p className="max-w-xl text-[13px] text-muted-foreground">
            For invoices paid by bank transfer, or whose automatic payment failed.
          </p>
        </div>
        <div className="flex items-center gap-3">
          <span className="text-[13px] text-muted-foreground">{kindEnabled ? 'On' : 'Off'}</span>
          <Switch
            checked={kindEnabled}
            disabled={!canManage || !entityId || settingsUnavailable || !settings}
            onCheckedChange={enabled =>
              entityId &&
              kindMut.mutate({
                invoicingEntityId: entityId,
                kind: EmailKind.PAYMENT_REMINDER,
                enabled,
              })
            }
            aria-label="Payment reminder emails"
          />
        </div>
      </div>

      {entityId && !settingsUnavailable && (
        <ReminderSchedule entityId={entityId} settings={settings} disabled={!canManage} />
      )}

      {unavailable && (
        <p className="text-[13px] text-muted-foreground">
          Payment reminders are not available on this server yet.
        </p>
      )}

      {listQuery.isLoading && (
        <div className="space-y-2">
          <Skeleton height={72} width="100%" />
          <Skeleton height={72} width="100%" />
        </div>
      )}

      <div className="flex flex-col gap-2">
        {sequences.map(sequence => {
          const index = ordered.findIndex(s => s.id === sequence.id)
          const title = sequence.isDefault ? DEFAULT_TITLE : sequence.name
          // Only judged once the settings (and so the enabled locales) are known.
          const missing = settings ? sequenceMissingLocales(sequence.steps, locales) : []
          const audience = sequence.isDefault
            ? 'Every open invoice no other sequence claims'
            : audienceSummary(parseAudience(sequence.audience), audienceNames) || 'No conditions'
          return (
            <div
              key={sequence.id}
              className={cn(
                'flex items-start gap-4 rounded-lg border border-border bg-card px-4 py-3',
                !sequence.enabled && 'opacity-70'
              )}
            >
              {!sequence.isDefault && (
                <div className="flex flex-col">
                  <Button
                    type="button"
                    variant="ghost"
                    size="sm"
                    className="h-6 w-6 p-0"
                    disabled={controlsDisabled || index <= 0 || reorderMut.isPending}
                    onClick={() => move(index, -1)}
                    aria-label="Move up"
                  >
                    <ArrowUp size={13} />
                  </Button>
                  <Button
                    type="button"
                    variant="ghost"
                    size="sm"
                    className="h-6 w-6 p-0"
                    disabled={
                      controlsDisabled || index >= ordered.length - 1 || reorderMut.isPending
                    }
                    onClick={() => move(index, 1)}
                    aria-label="Move down"
                  >
                    <ArrowDown size={13} />
                  </Button>
                </div>
              )}
              <div className="min-w-0 flex-1 space-y-1">
                <div className="flex flex-wrap items-center gap-2 text-[13px] font-medium">
                  {title}
                  {sequence.isDefault && (
                    <Badge variant="outline" size="sm" className="font-normal">
                      Default
                    </Badge>
                  )}
                  {!sequence.enabled && (
                    <Badge variant="secondary" size="sm" className="font-normal">
                      Disabled
                    </Badge>
                  )}
                  {missing.length > 0 && (
                    <Badge asChild variant="warning" size="sm" className="font-normal">
                      <button
                        type="button"
                        onClick={() => setSequenceParam(sequence.id)}
                        title="Steps with custom copy need it in every enabled language"
                      >
                        <AlertTriangle size={11} />
                        {localeListLabel(missing)} copy missing
                      </button>
                    </Badge>
                  )}
                </div>
                <div className="truncate text-xs text-muted-foreground" title={audience}>
                  {audience}
                </div>
                <div className="text-xs text-muted-foreground">
                  {stepsSummary(sequence)}
                  {sequence.inFlightInvoices > 0 &&
                    ` · ${inFlightLabel(sequence.inFlightInvoices)}`}
                </div>
              </div>
              <div className="flex shrink-0 items-center gap-1.5">
                <Button
                  type="button"
                  variant="ghost"
                  size="sm"
                  hasIcon
                  className="h-7 text-[13px]"
                  onClick={() => setSequenceParam(sequence.id)}
                >
                  <Pencil size={13} />
                  {canManage ? 'Edit' : 'View'}
                </Button>
                {!sequence.isDefault && canManage && (
                  <Button
                    type="button"
                    variant="ghost"
                    size="sm"
                    className="h-7 w-7 p-0 text-muted-foreground"
                    disabled={controlsDisabled || archiveMut.isPending}
                    onClick={() => archive(sequence)}
                    aria-label="Archive sequence"
                  >
                    <Archive size={13} />
                  </Button>
                )}
                <Switch
                  checked={sequence.enabled}
                  disabled={controlsDisabled || updateMut.isPending}
                  onCheckedChange={v => toggleSequence(sequence, v)}
                  aria-label={`${title} enabled`}
                />
              </div>
            </div>
          )
        })}
      </div>

      <div className="flex flex-wrap items-center justify-between gap-3">
        <p className="max-w-xl text-[13px] text-muted-foreground">{DISABLED_HELP}</p>
        {canManage && (
          <Button
            type="button"
            size="sm"
            hasIcon
            disabled={controlsDisabled}
            onClick={() => setCreating(true)}
          >
            <PlusIcon size={14} />
            New sequence
          </Button>
        )}
      </div>

      {entityId && (
        <SequenceSheet
          open={creating || editing !== null}
          onOpenChange={open => {
            if (open) return
            setCreating(false)
            setSequenceParam('')
          }}
          entityId={entityId}
          locales={locales}
          defaultCurrency={currentEntity?.accountingCurrency}
          sequence={creating ? null : editing}
        />
      )}
    </div>
  )
}
