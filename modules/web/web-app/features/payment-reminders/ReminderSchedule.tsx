import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { useMemo } from 'react'
import { toast } from 'sonner'

import { Combobox } from '@/components/Combobox'
import { emailSettingsQueryKey } from '@/features/customer-emails/EmailKindsView'
import {
  previewEmail,
  previewInvoiceEmail,
  updateEmailSettings,
} from '@/rpc/api/customeremails/v1/customeremails-CustomerEmailsService_connectquery'
import { EmailSettings } from '@/rpc/api/customeremails/v1/models_pb'

const NO_TIMEZONE = '__utc'

export const hourLabel = (hour: number) => `${String(hour).padStart(2, '0')}:00`

const supportedTimezones = (): string[] => {
  try {
    return Intl.supportedValuesOf('timeZone')
  } catch {
    return ['UTC']
  }
}

interface Props {
  entityId: string
  settings?: EmailSettings
  disabled?: boolean
}

// Send hour + timezone of the entity's reminders; each change saves on its own (patch semantics).
export const ReminderSchedule = ({ entityId, settings, disabled }: Props) => {
  const queryClient = useQueryClient()

  const timezoneOptions = useMemo(
    () => [
      { label: 'UTC (not set)', value: NO_TIMEZONE },
      ...supportedTimezones().map(tz => ({ label: tz.replace(/_/g, ' '), value: tz })),
    ],
    []
  )

  const updateMut = useMutation(updateEmailSettings, {
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: emailSettingsQueryKey(entityId) })
      await Promise.all([
        queryClient.invalidateQueries({
          queryKey: createConnectQueryKey({ schema: previewEmail, cardinality: undefined }),
        }),
        queryClient.invalidateQueries({
          queryKey: createConnectQueryKey({ schema: previewInvoiceEmail, cardinality: undefined }),
        }),
      ])
      toast.success('Reminder schedule updated')
    },
    onError: err => toast.error(err.rawMessage || 'Could not update the reminder schedule'),
  })

  const hour = settings?.reminderSendHour ?? 9
  const timezone = settings?.timezone ?? ''
  const locked = disabled || !settings || updateMut.isPending

  return (
    <div className="flex flex-col gap-2 rounded-lg border border-border p-4">
      <div className="text-[13px] font-medium">Schedule</div>
      <div className="flex flex-wrap items-center gap-2 text-[13px] text-muted-foreground">
        <span>Send reminders at</span>
        <Select
          value={String(hour)}
          disabled={locked}
          onValueChange={v =>
            updateMut.mutate({ invoicingEntityId: entityId, data: { reminderSendHour: Number(v) } })
          }
        >
          <SelectTrigger className="h-8 w-[92px]" aria-label="Send time">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {Array.from({ length: 24 }, (_, h) => (
              <SelectItem key={h} value={String(h)}>
                {hourLabel(h)}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        <span>in</span>
        <div className="w-[240px]">
          <Combobox
            hasSearch
            unit="timezone"
            placeholder="UTC (not set)"
            options={timezoneOptions}
            value={timezone || NO_TIMEZONE}
            onChange={v => {
              // Combobox has no disabled state; ignore picks while read-only or saving.
              if (locked || v === (timezone || NO_TIMEZONE)) return
              updateMut.mutate({
                invoicingEntityId: entityId,
                data: { timezone: v === NO_TIMEZONE ? '' : v },
              })
            }}
          />
        </div>
      </div>
      <p className="text-xs text-muted-foreground">
        The customer&apos;s day is judged in this timezone, so a reminder never lands in the middle
        of the night.
      </p>
    </div>
  )
}
