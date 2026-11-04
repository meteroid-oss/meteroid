import { useMutation } from '@connectrpc/connect-query'
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  Badge,
  Button,
  Label,
  RadioGroup,
  RadioGroupItem,
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
  cn,
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { useState } from 'react'
import { toast } from 'sonner'

import { ValuesTagInput } from '@/components/form/ValuesTagInput'
import { usePermissions } from '@/hooks/usePermissions'
import { useTenant } from '@/hooks/useTenant'
import { EmailDeliveryMode, TenantEnvironmentEnum } from '@/rpc/api/tenants/v1/models_pb'
import { updateTenant } from '@/rpc/api/tenants/v1/tenants-TenantsService_connectquery'

const MAX_TEST_RECIPIENTS = 50

// Same shape the server accepts: an address, or @domain, with a dotted domain.
const EMAIL_RE = /^[^\s@]+@[^\s@]+\.[^\s@]+$/
const DOMAIN_RE = /^@[^\s@]+\.[^\s@]+$/

const isValidRecipient = (value: string) => EMAIL_RE.test(value) || DOMAIN_RE.test(value)

const MODES: { value: EmailDeliveryMode; label: string; description: string }[] = [
  { value: EmailDeliveryMode.LIVE, label: 'Live', description: 'Emails go to your customers.' },
  {
    value: EmailDeliveryMode.TEST,
    label: 'Test',
    description:
      'Only your team and the test recipients receive emails. Others are recorded as skipped on the invoice or quote.',
  },
  {
    value: EmailDeliveryMode.OFF,
    label: 'Off',
    description:
      'No customer emails are sent. Each is recorded as skipped on the invoice or quote; payment reminders are paused.',
  },
]

export const DeliveryModeBadge = ({ mode }: { mode: EmailDeliveryMode }) => {
  switch (mode) {
    case EmailDeliveryMode.LIVE:
      return (
        <Badge variant="success" size="sm">
          Live
        </Badge>
      )
    case EmailDeliveryMode.TEST:
      return (
        <Badge variant="secondary" size="sm">
          Test mode
        </Badge>
      )
    case EmailDeliveryMode.OFF:
      return (
        <Badge variant="warning" size="sm">
          Off
        </Badge>
      )
    default:
      return null
  }
}

const strip = (mode: EmailDeliveryMode, testRecipients: number) => {
  switch (mode) {
    case EmailDeliveryMode.LIVE:
      return {
        className: 'border-border bg-card',
        text: 'Customer emails go to your customers.',
      }
    case EmailDeliveryMode.TEST:
      return {
        className: 'border-border bg-muted/50',
        text:
          testRecipients > 0
            ? `Only your team and ${testRecipients} test recipient${testRecipients === 1 ? '' : 's'} receive customer emails.`
            : 'Only your team receives customer emails.',
      }
    case EmailDeliveryMode.OFF:
      return {
        className: 'border-warning/40 bg-warning/10',
        text: 'No customer emails are sent, and payment reminders are paused.',
      }
    default:
      return undefined
  }
}

const sameList = (a: string[], b: string[]) =>
  a.length === b.length && a.every((v, i) => v === b[i])

/** The workspace's delivery mode as a one-line status, edited in a sheet. */
export const DeliveryView = () => {
  const { tenant } = useTenant()
  const canManage = usePermissions().has('settings:manage')
  const [open, setOpen] = useState(false)

  if (!tenant) return null
  const status = strip(tenant.emailDeliveryMode, tenant.emailTestRecipients.length)
  if (!status) return null

  return (
    <div
      className={cn(
        'flex flex-wrap items-center gap-x-3 gap-y-1 rounded-lg border px-3 py-2 text-[13px]',
        status.className
      )}
    >
      <span className="font-medium">Delivery</span>
      <DeliveryModeBadge mode={tenant.emailDeliveryMode} />
      <span className="min-w-0 text-muted-foreground">{status.text}</span>
      {canManage && (
        <Button variant="outline" size="sm" className="ml-auto" onClick={() => setOpen(true)}>
          Change
        </Button>
      )}
      <Sheet open={open} onOpenChange={setOpen}>
        <SheetContent>
          <SheetHeader>
            <SheetTitle>Email delivery</SheetTitle>
            <SheetDescription>
              Where customer emails go. Applies to every invoicing entity in this workspace; alerts
              to your team are not affected.
            </SheetDescription>
          </SheetHeader>
          <DeliveryForm onClose={() => setOpen(false)} />
        </SheetContent>
      </Sheet>
    </div>
  )
}

// Mounted only while the sheet is open, so drafts start from the saved values each time.
const DeliveryForm = ({ onClose }: { onClose: () => void }) => {
  const { tenant } = useTenant()
  const queryClient = useQueryClient()

  const savedMode = tenant?.emailDeliveryMode ?? EmailDeliveryMode.TEST
  const savedRecipients = tenant?.emailTestRecipients ?? []
  const isSandbox = tenant?.environment === TenantEnvironmentEnum.SANDBOX
  const isProduction = tenant?.environment === TenantEnvironmentEnum.PRODUCTION
  // Only production can go live; a non-production workspace already live keeps it.
  const keptLive = !isProduction && savedMode === EmailDeliveryMode.LIVE
  const liveDisabled = !isProduction && !keptLive
  const liveNote = isSandbox
    ? 'A sandbox never emails customers.'
    : 'Only production workspaces can go live.'

  const [mode, setMode] = useState<EmailDeliveryMode>(savedMode)
  const [recipients, setRecipients] = useState<string[]>(savedRecipients)
  const [confirmLive, setConfirmLive] = useState(false)

  const updateMut = useMutation(updateTenant, {
    onSuccess: async () => {
      await queryClient.invalidateQueries()
      toast.success('Email delivery updated')
      onClose()
    },
    onError: err => toast.error(err.rawMessage || 'Could not update email delivery'),
  })

  const modeChanged = mode !== savedMode
  const recipientsChanged = !sameList(recipients, savedRecipients)
  const invalid = recipients.filter(r => !isValidRecipient(r))
  // The list is only edited (and its errors only shown) in Test; elsewhere it is not sent.
  const sendRecipients = recipientsChanged && mode === EmailDeliveryMode.TEST
  const canSave =
    (modeChanged || sendRecipients) &&
    !(sendRecipients && invalid.length > 0) &&
    !(liveDisabled && mode === EmailDeliveryMode.LIVE) &&
    !updateMut.isPending

  const save = () =>
    updateMut.mutate({
      data: {
        emailDeliveryMode: modeChanged ? mode : undefined,
        emailTestRecipients: sendRecipients ? { recipients } : undefined,
      },
    })

  const onSave = () => {
    if (modeChanged && mode === EmailDeliveryMode.LIVE && isProduction) {
      setConfirmLive(true)
      return
    }
    save()
  }

  const onRecipientsChange = (values: string[]) => {
    const normalized = values.map(v => v.trim().toLowerCase()).filter(Boolean)
    setRecipients(Array.from(new Set(normalized)).slice(0, MAX_TEST_RECIPIENTS))
  }

  return (
    <>
      <div className="flex flex-1 flex-col gap-5 overflow-y-auto py-4">
        <RadioGroup
          value={String(mode)}
          onValueChange={v => setMode(Number(v) as EmailDeliveryMode)}
          className="gap-2"
        >
          {MODES.map(m => {
            const disabled = m.value === EmailDeliveryMode.LIVE && liveDisabled
            const id = `email-delivery-${m.value}`
            return (
              <Label
                key={m.value}
                htmlFor={id}
                className={cn(
                  'flex items-start gap-2.5 rounded-lg border border-border bg-card p-3 font-normal',
                  mode === m.value && 'border-primary',
                  disabled ? 'cursor-not-allowed opacity-60' : 'cursor-pointer'
                )}
              >
                <RadioGroupItem
                  value={String(m.value)}
                  id={id}
                  disabled={disabled}
                  className="mt-0.5"
                />
                <span className="space-y-0.5">
                  <span className="block text-[13px] font-medium">{m.label}</span>
                  <span className="block text-xs leading-snug text-muted-foreground">
                    {disabled ? liveNote : m.description}
                  </span>
                </span>
              </Label>
            )
          })}
        </RadioGroup>

        {keptLive && (
          <p className="text-xs text-muted-foreground">
            This non-production workspace delivers to customers from before Live became
            production-only. Once switched to Test or Off, it cannot go back to Live.
          </p>
        )}

        {mode === EmailDeliveryMode.TEST && (
          <div className="space-y-1.5">
            <Label className="text-[13px]">Test recipients</Label>
            <div className="rounded-md border border-border p-1">
              <ValuesTagInput
                value={recipients}
                onChange={onRecipientsChange}
                placeholder="name@example.com or @example.com, then press Enter"
              />
            </div>
            {invalid.length > 0 ? (
              <p className="text-xs text-destructive">
                Not an email address or @domain: {invalid.join(', ')}
              </p>
            ) : (
              <p className="text-xs text-muted-foreground">
                Email addresses, or @domain for a whole domain (up to {MAX_TEST_RECIPIENTS}).
                Members of your organization always receive test emails.
              </p>
            )}
          </div>
        )}
      </div>

      <div className="mt-auto flex justify-end gap-2 border-t border-border pt-4">
        <Button type="button" variant="secondary" onClick={onClose}>
          Cancel
        </Button>
        <Button type="button" disabled={!canSave} onClick={onSave}>
          Save
        </Button>
      </div>

      <AlertDialog open={confirmLive} onOpenChange={setConfirmLive}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Switch customer emails to Live?</AlertDialogTitle>
            <AlertDialogDescription>
              Customer emails will be delivered to your customers&apos; real addresses.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction onClick={save}>Go live</AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  )
}
