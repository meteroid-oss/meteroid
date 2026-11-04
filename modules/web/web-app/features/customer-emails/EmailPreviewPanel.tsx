import { Code } from '@connectrpc/connect'
import { useMutation } from '@connectrpc/connect-query'
import { Button, cn, Skeleton } from '@md/ui'
import { keepPreviousData } from '@tanstack/react-query'
import { AlertTriangle, Paperclip, Send } from 'lucide-react'
import { useEffect, useMemo, useState } from 'react'
import { toast } from 'sonner'

import { EmailHtmlFrame } from '@/components/EmailHtmlFrame'
import { Gate } from '@/components/Gate'
import { INVOICE_LANGUAGE_OPTIONS } from '@/components/InvoiceLanguageSelect'
import { useDebounceValue } from '@/hooks/useDebounce'
import { useQuery } from '@/lib/connectrpc'
import {
  getEmailSettings,
  previewEmail,
  sendTestEmail,
} from '@/rpc/api/customeremails/v1/customeremails-CustomerEmailsService_connectquery'
import { EmailKind } from '@/rpc/api/customeremails/v1/models_pb'

import { TestBannerNote } from './TestBannerNote'
import { enabledLocales } from './locales'

export interface EmailPreviewPanelProps {
  entityId: string
  kind: EmailKind
  locale?: string
  draft?: { subject: string; body: string }
  offsetDays?: number
  onLocaleChange?: (locale: string) => void
  className?: string
}

const PREVIEW_DEBOUNCE_MS = 400

export const localeLabel = (locale: string): string =>
  INVOICE_LANGUAGE_OPTIONS.find(o => o.value === locale)?.label ?? locale

const HeaderRow = ({ label, value }: { label: string; value?: string }) =>
  value ? (
    <div className="flex gap-2 text-xs">
      <span className="w-14 shrink-0 font-medium text-muted-foreground">{label}</span>
      <span className="min-w-0 break-all text-foreground">{value}</span>
    </div>
  ) : null

export const EmailPreviewPanel = ({
  entityId,
  kind,
  locale,
  draft,
  offsetDays,
  onLocaleChange,
  className,
}: EmailPreviewPanelProps) => {
  const [pickedLocale, setPickedLocale] = useState<string | undefined>()

  // The picker offers the entity's enabled locales only (none while settings load).
  const settingsQuery = useQuery(
    getEmailSettings,
    { invoicingEntityId: entityId },
    { retry: (count, err) => err.code !== Code.Unimplemented && count < 2 }
  )
  const settings = settingsQuery.data?.settings
  const locales = useMemo(() => enabledLocales(settings), [settings])

  // A pick that is no longer enabled falls back to the default.
  useEffect(() => {
    if (pickedLocale && locales.length > 0 && !locales.includes(pickedLocale)) {
      setPickedLocale(undefined)
    }
  }, [pickedLocale, locales])

  // Absent locale = the entity's effective locale, resolved server-side; a locale that
  // is not enabled (any more) previews the default instead.
  const requested = locale ?? pickedLocale
  const selectedLocale =
    requested && locales.length > 0 && !locales.includes(requested) ? locales[0] : requested

  const subject = draft?.subject
  const body = draft?.body
  const request = useMemo(
    () => ({
      invoicingEntityId: entityId,
      kind,
      locale: selectedLocale,
      draft: subject !== undefined && body !== undefined ? { subject, body } : undefined,
      offsetDays,
    }),
    [entityId, kind, selectedLocale, subject, body, offsetDays]
  )
  const debounced = useDebounceValue(request, PREVIEW_DEBOUNCE_MS)

  const query = useQuery(previewEmail, debounced, {
    placeholderData: keepPreviousData,
    retry: (count, err) => err.code !== Code.Unimplemented && count < 2,
  })
  const email = query.data?.email
  const unavailable = query.error?.code === Code.Unimplemented
  const shownLocale = selectedLocale ?? email?.locale

  const testMut = useMutation(sendTestEmail, {
    onSuccess: res =>
      res.delivered
        ? toast.success(`Test email sent to ${res.to}`)
        : toast.warning(`Email delivery is disabled on this server — nothing was sent to ${res.to}`),
    onError: err => toast.error(err.rawMessage || 'Could not send the test email'),
  })

  const pickLocale = (value: string) => {
    setPickedLocale(value)
    onLocaleChange?.(value)
  }

  return (
    <div className={cn('flex flex-col gap-3 rounded-lg border border-border bg-card p-4', className)}>
      <div className="flex flex-wrap items-center justify-between gap-2">
        {locales.length > 0 ? (
          <div className="inline-flex flex-wrap items-center gap-1 rounded-lg border border-border bg-muted/40 p-0.5">
            {locales.map(value => (
              <button
                key={value}
                type="button"
                onClick={() => pickLocale(value)}
                className={cn(
                  'rounded-md px-2.5 py-1 text-xs font-medium transition-colors',
                  value === shownLocale
                    ? 'bg-background text-foreground shadow-sm'
                    : 'text-muted-foreground hover:text-foreground'
                )}
              >
                {localeLabel(value)}
              </button>
            ))}
          </div>
        ) : (
          <span />
        )}
        <Gate perm="settings:manage">
          <Button
            type="button"
            variant="outline"
            size="sm"
            hasIcon
            disabled={testMut.isPending || unavailable}
            onClick={() => testMut.mutate(request)}
          >
            <Send size={12} />
            Send test
          </Button>
        </Gate>
      </div>

      {query.isLoading && (
        <div className="space-y-2">
          <Skeleton height={14} width={260} />
          <Skeleton height={14} width={200} />
          <Skeleton height={320} width="100%" />
        </div>
      )}

      {query.isError && (
        <div className="py-8 text-center text-sm text-muted-foreground">
          {unavailable
            ? 'Previews are not available on this server yet.'
            : query.error.rawMessage || 'Couldn’t render this preview.'}
        </div>
      )}

      {email && (
        <>
          <TestBannerNote />
          <div className="space-y-1">
            <div className="text-sm font-medium">{email.subject}</div>
            <HeaderRow label="From" value={email.from} />
            <HeaderRow label="Reply-to" value={email.replyTo} />
            <HeaderRow label="Bcc" value={email.bcc.length ? email.bcc.join(', ') : undefined} />
          </div>
          {email.attachmentNames.length > 0 && (
            <div className="flex flex-wrap gap-2">
              {email.attachmentNames.map(name => (
                <span
                  key={name}
                  className="inline-flex items-center gap-1.5 rounded border px-2 py-1 text-xs text-muted-foreground"
                >
                  <Paperclip className="h-3.5 w-3.5" />
                  {name}
                </span>
              ))}
            </div>
          )}
          {email.unknownVariables.length > 0 && (
            <div className="flex items-start gap-1.5 text-[13px] text-amber-700 dark:text-amber-500">
              <AlertTriangle size={14} className="mt-0.5 shrink-0" />
              <span>
                Unknown variable{email.unknownVariables.length === 1 ? '' : 's'}:{' '}
                {email.unknownVariables.map(v => `{{${v}}}`).join(', ')}
              </span>
            </div>
          )}
          <EmailHtmlFrame
            html={email.bodyHtml}
            className={cn('h-[520px]', query.isPlaceholderData && 'opacity-60')}
          />
        </>
      )}
    </div>
  )
}
