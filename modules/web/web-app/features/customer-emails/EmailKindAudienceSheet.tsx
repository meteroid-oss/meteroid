import { Code } from '@connectrpc/connect'
import { useMutation } from '@connectrpc/connect-query'
import {
  Button,
  Label,
  RadioGroup,
  RadioGroupItem,
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { useMemo, useState } from 'react'
import { toast } from 'sonner'

import { useDebounceValue } from '@/hooks/useDebounce'
import { usePermissions } from '@/hooks/usePermissions'
import { useQuery } from '@/lib/connectrpc'
import {
  clearEmailKindAudience,
  previewEmailKindAudience,
  setEmailKindAudience,
} from '@/rpc/api/customeremails/v1/customeremails-CustomerEmailsService_connectquery'
import { GetEmailSettingsResponse } from '@/rpc/api/customeremails/v1/customeremails_pb'
import {
  EmailAudienceMode,
  EmailKind,
  EmailKindAudiencePreview,
  EmailKindInfo,
  EmailSettings,
} from '@/rpc/api/customeremails/v1/models_pb'

import { AudienceBuilder } from './audience/AudienceBuilder'
import {
  type AudienceLabelContext,
  audienceSummary,
  groupDepth,
  MAX_GROUP_DEPTH,
  parseAudience,
  sanitizeAudience,
  serializeAudience,
} from './audience/model'

import type { AudiencePredicate } from './audience/types'

type SendTo = 'everyone' | 'exclude' | 'only'

const MODE_OF: Record<Exclude<SendTo, 'everyone'>, EmailAudienceMode> = {
  exclude: EmailAudienceMode.EXCLUDE,
  only: EmailAudienceMode.ONLY,
}

const sendToOf = (info: EmailKindInfo): SendTo =>
  info.audience?.mode === EmailAudienceMode.EXCLUDE
    ? 'exclude'
    : info.audience?.mode === EmailAudienceMode.ONLY
      ? 'only'
      : 'everyone'

export const sendToSummary = (info: EmailKindInfo, ctx: AudienceLabelContext): string => {
  const sendTo = sendToOf(info)
  if (sendTo === 'everyone') return 'Everyone'
  const label = audienceSummary(parseAudience(info.audience?.audience), ctx) || 'no conditions'
  return sendTo === 'exclude' ? `Everyone except: ${label}` : `Only: ${label}`
}

const SEND_TO_OPTIONS: { value: SendTo; label: string; help: string }[] = [
  { value: 'everyone', label: 'Everyone', help: 'Every email of this kind is sent.' },
  {
    value: 'exclude',
    label: 'Everyone except',
    help: 'Emails matching the conditions are skipped.',
  },
  { value: 'only', label: 'Only', help: 'Only emails matching the conditions are sent.' },
]

const countLabel = (p: EmailKindAudiencePreview, customerLevel: boolean) => {
  if (customerLevel) {
    return p.scannedCustomers === 0
      ? 'No customers yet to count against.'
      : `${p.receivingCustomers} of ${p.scannedCustomers} customers would get it · ${p.matchedCustomers} match the conditions`
  }
  return p.scannedInvoices === 0
    ? 'No finalized invoices yet to count against.'
    : `${p.receivingInvoices} of the last ${p.scannedInvoices} finalized invoices would get it (${p.receivingCustomers} of ${p.scannedCustomers} customers) · ${p.matchedInvoices} match the conditions`
}

interface Props {
  open: boolean
  onOpenChange: (open: boolean) => void
  entityId: string
  kindLabel: string
  info: EmailKindInfo
  settingsQueryKey: readonly unknown[]
  defaultCurrency?: string
}

export const EmailKindAudienceSheet = ({ open, onOpenChange, kindLabel, ...rest }: Props) => (
  <Sheet open={open} onOpenChange={onOpenChange}>
    <SheetContent size="large">
      <SheetHeader>
        <SheetTitle>{kindLabel}: send to</SheetTitle>
        <SheetDescription>
          Checked on current data each time this email is about to go out, so a change applies from
          the next email. Resends and test emails always go out.
        </SheetDescription>
      </SheetHeader>
      <AudienceForm onClose={() => onOpenChange(false)} kindLabel={kindLabel} {...rest} />
    </SheetContent>
  </Sheet>
)

// Mounted with the sheet content, so each opening starts from the stored rule.
const AudienceForm = ({
  onClose,
  entityId,
  kindLabel,
  info,
  settingsQueryKey,
  defaultCurrency,
}: Omit<Props, 'open' | 'onOpenChange'> & { onClose: () => void }) => {
  const queryClient = useQueryClient()
  const canManage = usePermissions().has('settings:manage')
  const customerLevel = info.customerLevelAudience || info.kind === EmailKind.QUOTE_READY
  const [sendTo, setSendTo] = useState<SendTo>(() => sendToOf(info))
  const [predicates, setPredicates] = useState<AudiencePredicate[]>(() =>
    parseAudience(info.audience?.audience)
  )
  const [error, setError] = useState<string | null>(null)

  const clean = useMemo(() => sanitizeAudience(predicates), [predicates])
  const audienceJson = useDebounceValue(serializeAudience(clean), 400)
  const preview = useQuery(
    previewEmailKindAudience,
    {
      invoicingEntityId: entityId,
      kind: info.kind,
      mode: sendTo === 'everyone' ? EmailAudienceMode.EXCLUDE : MODE_OF[sendTo],
      audience: audienceJson,
    },
    {
      enabled: sendTo !== 'everyone' && clean.length > 0,
      retry: (count, err) => err.code !== Code.Unimplemented && count < 2,
    }
  )

  const onSaved = (settings: EmailSettings | undefined, message: string) => {
    if (settings) {
      queryClient.setQueriesData<GetEmailSettingsResponse>({ queryKey: settingsQueryKey }, old =>
        old ? { ...old, settings } : old
      )
    }
    queryClient.invalidateQueries({ queryKey: settingsQueryKey })
    onClose()
    toast.success(message)
  }
  const onFailed = (err: { code: Code; rawMessage: string }) =>
    toast.error(
      err.code === Code.Unimplemented
        ? 'Send-to rules are not available on this server yet.'
        : err.rawMessage || 'Could not save who gets this email'
    )
  const setMut = useMutation(setEmailKindAudience, {
    onSuccess: res => onSaved(res.settings, `${kindLabel}: send-to rule saved`),
    onError: onFailed,
  })
  const clearMut = useMutation(clearEmailKindAudience, {
    onSuccess: res => onSaved(res.settings, `${kindLabel}: sent to everyone`),
    onError: onFailed,
  })
  const submitting = setMut.isPending || clearMut.isPending

  const save = () => {
    if (sendTo === 'everyone') {
      if (info.audience) clearMut.mutate({ invoicingEntityId: entityId, kind: info.kind })
      else onClose()
      return
    }
    if (clean.length === 0) {
      setError('Add at least one condition, or send to everyone.')
      return
    }
    if (clean.some(p => groupDepth(p) > MAX_GROUP_DEPTH)) {
      setError(`Groups nest at most ${MAX_GROUP_DEPTH} deep`)
      return
    }
    setMut.mutate({
      invoicingEntityId: entityId,
      kind: info.kind,
      mode: MODE_OF[sendTo],
      audience: serializeAudience(clean),
    })
  }

  const previewData = preview.data?.preview

  return (
    <>
      <fieldset disabled={!canManage} className="flex flex-1 flex-col gap-5 overflow-y-auto py-4">
        <RadioGroup value={sendTo} onValueChange={v => setSendTo(v as SendTo)}>
          {SEND_TO_OPTIONS.map(o => (
            <div key={o.value} className="flex items-start space-x-2">
              <RadioGroupItem value={o.value} id={`send-to-${o.value}`} className="mt-1" />
              <Label htmlFor={`send-to-${o.value}`} className="cursor-pointer text-sm font-normal">
                {o.label}
                <span className="block text-xs text-muted-foreground">{o.help}</span>
              </Label>
            </div>
          ))}
        </RadioGroup>

        {sendTo !== 'everyone' && (
          <div className="space-y-2 rounded-lg border border-border p-3">
            <AudienceBuilder
              predicates={predicates}
              onChange={v => {
                setPredicates(v)
                setError(null)
              }}
              defaultCurrency={defaultCurrency}
              customerLevelOnly={customerLevel}
              emptyLabel="no conditions yet"
              help={
                customerLevel
                  ? 'Quotes have no invoice yet: only customer, country and customer property conditions apply.'
                  : 'Conditions are matched against the invoice the email is about (for credit notes, the credited invoice).'
              }
            />
            {error && <p className="text-xs text-destructive">{error}</p>}
            <p className="text-[13px] text-muted-foreground">
              {clean.length === 0
                ? 'Add a condition to see how many emails it would affect.'
                : preview.error?.code === Code.Unimplemented
                  ? 'Live counts are not available on this server yet.'
                  : preview.isError
                    ? preview.error.rawMessage || 'Could not count matching emails.'
                    : previewData
                      ? countLabel(previewData, customerLevel)
                      : 'Counting…'}
            </p>
          </div>
        )}
      </fieldset>
      <div className="mt-auto flex justify-end gap-2 border-t border-border pt-4">
        <Button type="button" variant="secondary" onClick={onClose}>
          Cancel
        </Button>
        <Button type="button" onClick={save} disabled={submitting || !canManage}>
          Save
        </Button>
      </div>
    </>
  )
}
