import { ConnectError } from '@connectrpc/connect'
import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query'
import { Button, Input, Skeleton, Switch } from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { X } from 'lucide-react'
import { FunctionComponent, useState } from 'react'
import { Link } from 'react-router'
import { toast } from 'sonner'

import { SettingsSection } from '@/features/settings/components/SettingsSection'
import { useBasePath } from '@/hooks/useBasePath'
import { usePermissions } from '@/hooks/usePermissions'
import { useQuery } from '@/lib/connectrpc'
import {
  getRenewalSettings,
  updateRenewalSettings,
} from '@/rpc/api/renewals/v1/renewals-RenewalsService_connectquery'

const MAX_CHECKPOINTS = 6

/** Days before the opt-out deadline (else the date) the team is notified, largest first. */
export const CheckpointsField = ({
  days,
  disabled,
  onChange,
}: {
  days: number[]
  disabled: boolean
  onChange: (days: number[]) => void
}) => {
  const [adding, setAdding] = useState('')
  const add = () => {
    const n = Number(adding)
    if (!Number.isInteger(n) || n < 1 || n > 365) {
      toast.error('Each notice is 1 to 365 days before')
      return
    }
    if (days.length >= MAX_CHECKPOINTS) {
      toast.error(`Up to ${MAX_CHECKPOINTS} notices`)
      return
    }
    setAdding('')
    if (!days.includes(n)) onChange([...days, n].sort((a, b) => b - a))
  }
  return (
    <div className="flex flex-wrap items-center gap-2">
      {days.length === 0 && <span className="text-sm text-muted-foreground">No notice</span>}
      {days.map(d => (
        <span
          key={d}
          className="inline-flex h-8 items-center gap-1 rounded-full bg-primary/10 pl-3 pr-1 text-sm font-medium"
        >
          {d} days
          <button
            type="button"
            aria-label={`Remove ${d} days`}
            disabled={disabled}
            className="flex h-6 w-6 items-center justify-center rounded-full hover:bg-primary/20"
            onClick={() => onChange(days.filter(x => x !== d))}
          >
            <X size={12} />
          </button>
        </span>
      ))}
      {!disabled && (
        <form
          className="flex items-center gap-1"
          onSubmit={e => {
            e.preventDefault()
            add()
          }}
        >
          <Input
            aria-label="Days before"
            type="number"
            min={1}
            max={365}
            placeholder="Days"
            className="h-8 w-20"
            value={adding}
            onChange={e => setAdding(e.target.value)}
          />
          <Button type="submit" size="sm" variant="secondary" disabled={!adding}>
            Add
          </Button>
        </form>
      )}
    </div>
  )
}

const ReminderField = ({
  days,
  disabled,
  onChange,
}: {
  days: number | undefined
  disabled: boolean
  onChange: (days: number | undefined) => void
}) => {
  const [draft, setDraft] = useState(String(days ?? 30))
  const on = days !== undefined
  const save = () => {
    const n = Number(draft)
    if (!Number.isInteger(n) || n < 1 || n > 365) {
      toast.error('A reminder is 1 to 365 days before')
      return
    }
    if (n !== days) onChange(n)
  }
  return (
    <div className="flex flex-wrap items-center gap-3">
      <Switch
        id="renewal-reminder"
        checked={on}
        disabled={disabled}
        onCheckedChange={checked => onChange(checked ? Number(draft) || 30 : undefined)}
      />
      <label htmlFor="renewal-reminder" className="text-sm">
        {on ? 'Email the customer' : 'Off'}
      </label>
      {on && (
        <form
          className="flex items-center gap-1"
          onSubmit={e => {
            e.preventDefault()
            save()
          }}
        >
          <Input
            aria-label="Days before the reminder"
            type="number"
            min={1}
            max={365}
            className="h-8 w-20"
            value={draft}
            disabled={disabled}
            onChange={e => setDraft(e.target.value)}
            onBlur={save}
          />
          <span className="text-sm text-muted-foreground">days before</span>
        </form>
      )}
    </div>
  )
}

export const RenewalsTab: FunctionComponent = () => {
  const queryClient = useQueryClient()
  const basePath = useBasePath()
  const canManage = usePermissions().has('settings:manage')
  const settings = useQuery(getRenewalSettings, {}).data?.settings
  const update = useMutation(updateRenewalSettings, {
    onSuccess: () => {
      toast.success('Saved')
      return queryClient.invalidateQueries({
        queryKey: createConnectQueryKey({ schema: getRenewalSettings, cardinality: undefined }),
      })
    },
    onError: (e: Error) =>
      toast.error(ConnectError.from(e).rawMessage || 'Something went wrong, please try again'),
  })
  return (
    <div className="flex max-w-3xl flex-col gap-6">
      <p className="text-sm text-muted-foreground">
        One policy for every renewal: a contract term&apos;s, or an annual or semiannual
        period&apos;s, an end date or a scheduled cancellation (with Retention). Days count to the
        opt-out deadline of a term with notice, else to the renewal or end date.
      </p>
      <SettingsSection
        divided={false}
        title="Team notices"
        description="Sent as the subscription.renewal_upcoming webhook, once at each checkpoint. A checkpoint already behind when a renewal is first tracked is skipped."
        action={
          <Link to={`${basePath}/settings/webhooks`} className="text-sm font-medium hover:underline">
            Webhook settings
          </Link>
        }
      >
        {settings ? (
          <CheckpointsField
            days={settings.checkpoints}
            disabled={!canManage}
            onChange={days => update.mutate({ checkpoints: { days } })}
          />
        ) : (
          <Skeleton className="h-8" />
        )}
      </SettingsSection>
      <SettingsSection
        title="Customer reminder"
        description="Emails the customer once before a renewal or an ending, never after the opt-out deadline. Its delivery and audience follow the Renewal reminder kind of Customer emails."
        action={
          <Link to={`${basePath}/settings/emails`} className="text-sm font-medium hover:underline">
            Customer emails
          </Link>
        }
      >
        {settings ? (
          <ReminderField
            days={settings.customerReminderDays}
            disabled={!canManage}
            onChange={days => update.mutate({ customerReminder: { days } })}
          />
        ) : (
          <Skeleton className="h-8" />
        )}
      </SettingsSection>
    </div>
  )
}

export default RenewalsTab
