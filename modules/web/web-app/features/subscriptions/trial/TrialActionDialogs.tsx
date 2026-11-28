import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query'
import {
  Button,
  DatePicker,
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  Form,
  GenericFormField,
  InputFormField,
  Label,
  RadioGroup,
  RadioGroupItem,
  Skeleton,
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { addDays, max, parseISO, startOfToday } from 'date-fns'
import { useId } from 'react'
import { useWatch } from 'react-hook-form'
import { toast } from 'sonner'
import { z } from 'zod'

import { invalidateSubscriptionBilling } from '@/features/subscriptions/invalidateSubscriptionBilling'
import { useZodForm } from '@/hooks/useZodForm'
import { useQuery } from '@/lib/connectrpc'
import { mapDatev2 } from '@/lib/mapping'
import { rpcErrorMessage } from '@/lib/rpcError'
import { formatDateOnly } from '@/lib/utils/date'
import { formatCurrency } from '@/lib/utils/numbers'
import { listSubscriptions } from '@/rpc/api/subscriptions/v1/subscriptions-SubscriptionsService_connectquery'
import {
  endTrial,
  extendTrial,
  previewEndTrial,
} from '@/rpc/api/trials/v1/trials-SubscriptionTrialsService_connectquery'

import { END_OUTCOME } from './trialFacts'

import type { QueryClient } from '@tanstack/react-query'

/** After a trial moved: what reads the subscription, its lists, and any query keyed by it. */
const refreshTrial = (queryClient: QueryClient, subscriptionId: string) =>
  Promise.all([
    invalidateSubscriptionBilling(queryClient, subscriptionId),
    queryClient.invalidateQueries({
      queryKey: createConnectQueryKey({ schema: listSubscriptions, cardinality: undefined }),
    }),
    queryClient.invalidateQueries({
      predicate: query => {
        const props = query.queryKey[1] as { input?: { subscriptionId?: unknown } } | undefined
        return props?.input?.subscriptionId === subscriptionId
      },
    }),
  ])

const extendSchema = z
  .object({
    by: z.enum(['days', 'date']),
    days: z.preprocess(
      value => (value === '' || value === null ? undefined : value),
      z
        .number()
        .int('Enter a whole number of days')
        .min(1, 'From 1 to 3650 days')
        .max(3650, 'From 1 to 3650 days')
        .optional()
    ),
    until: z.date().optional(),
  })
  .superRefine((value, ctx) => {
    if (value.by === 'days' && value.days === undefined) {
      ctx.addIssue({ code: 'custom', message: 'Enter a number of days', path: ['days'] })
    }
    if (value.by === 'date' && !value.until) {
      ctx.addIssue({ code: 'custom', message: 'Pick a date', path: ['until'] })
    }
  })

export const ExtendTrialDialog = ({
  subscriptionId,
  endsOn,
  onClose,
}: {
  subscriptionId: string
  /** The trial's current end, exclusive. */
  endsOn?: string
  onClose: () => void
}) => {
  const id = useId()
  const queryClient = useQueryClient()
  const methods = useZodForm({ schema: extendSchema, defaultValues: { by: 'days', days: 7 } })
  const by = useWatch({ control: methods.control, name: 'by' })
  // The new end comes after the current one and after today.
  const firstDay = addDays(max([startOfToday(), ...(endsOn ? [parseISO(endsOn)] : [])]), 1)
  const extend = useMutation(extendTrial, {
    onSuccess: async response => {
      await refreshTrial(queryClient, subscriptionId)
      toast.success(
        response.trial?.endsOn
          ? `Trial extended to ${formatDateOnly(response.trial.endsOn)}`
          : 'Trial extended'
      )
      onClose()
    },
    onError: error => toast.error(rpcErrorMessage(error, 'The trial could not be extended')),
  })

  return (
    <Dialog open onOpenChange={open => !open && onClose()}>
      <DialogContent className="max-h-[90vh] overflow-y-auto sm:max-w-[480px]">
        <DialogHeader>
          <DialogTitle>Extend trial</DialogTitle>
        </DialogHeader>
        <Form {...methods}>
          <form
            onSubmit={methods.handleSubmit(form =>
              extend.mutate({
                subscriptionId,
                to:
                  form.by === 'date' && form.until
                    ? { case: 'untilDate', value: mapDatev2(form.until) }
                    : { case: 'days', value: form.days ?? 0 },
              })
            )}
            className="space-y-4"
          >
            <GenericFormField
              control={methods.control}
              name="by"
              label="Extend"
              render={({ field }) => (
                <RadioGroup
                  value={field.value}
                  onValueChange={field.onChange}
                  className="flex h-8 items-center gap-4"
                >
                  <div className="flex items-center gap-2">
                    <RadioGroupItem value="days" id={`${id}-days`} />
                    <Label htmlFor={`${id}-days`} className="font-normal">
                      By days
                    </Label>
                  </div>
                  <div className="flex items-center gap-2">
                    <RadioGroupItem value="date" id={`${id}-date`} />
                    <Label htmlFor={`${id}-date`} className="font-normal">
                      To a date
                    </Label>
                  </div>
                </RadioGroup>
              )}
            />
            {by === 'days' ? (
              <InputFormField
                control={methods.control}
                name="days"
                label="Days added"
                type="number"
                inputMode="numeric"
                min={1}
                step={1}
                rightText="days"
              />
            ) : (
              <GenericFormField
                control={methods.control}
                name="until"
                label="Ends on"
                render={({ field }) => (
                  <DatePicker
                    mode="single"
                    captionLayout="dropdown"
                    className="w-full"
                    placeholder="Pick a date"
                    clearable
                    date={field.value}
                    onSelect={field.onChange}
                    disabled={{ before: firstDay }}
                  />
                )}
              />
            )}
            {endsOn && (
              <div className="flex items-center justify-between text-[13px]">
                <span className="text-muted-foreground">Ends now</span>
                <span>{formatDateOnly(endsOn)}</span>
              </div>
            )}
            <DialogFooter>
              <Button type="button" variant="outline" onClick={onClose}>
                Cancel
              </Button>
              <Button type="submit" variant="primary" disabled={extend.isPending}>
                Extend
              </Button>
            </DialogFooter>
          </form>
        </Form>
      </DialogContent>
    </Dialog>
  )
}

/** Ending a trial today applies its end action now; a trial awaiting a decision converts. */
export const EndTrialDialog = ({
  subscriptionId,
  converting,
  onClose,
}: {
  subscriptionId: string
  /** Awaiting a decision: ending it converts it. */
  converting: boolean
  onClose: () => void
}) => {
  const queryClient = useQueryClient()
  const preview = useQuery(previewEndTrial, { subscriptionId })
  const end = useMutation(endTrial, {
    onSuccess: async () => {
      await refreshTrial(queryClient, subscriptionId)
      toast.success(converting ? 'Trial converted' : 'Trial ended')
      onClose()
    },
    onError: error => toast.error(rpcErrorMessage(error, 'The trial could not be ended')),
  })
  const result = preview.data
  const title = converting ? 'Convert now' : 'End trial now'

  return (
    <Dialog open onOpenChange={open => !open && onClose()}>
      <DialogContent className="max-h-[90vh] overflow-y-auto sm:max-w-[480px]">
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
        </DialogHeader>
        {preview.isLoading ? (
          <div className="space-y-2">
            <Skeleton height={16} />
            <Skeleton height={16} />
          </div>
        ) : preview.error || !result ? (
          <p className="text-xs text-destructive">
            {rpcErrorMessage(preview.error, 'The trial’s end could not be previewed')}
          </p>
        ) : (
          <div className="flex flex-col gap-2 text-[13px]">
            <Row label="Outcome" value={END_OUTCOME[result.outcome] ?? '—'} />
            <Row label="On" value={formatDateOnly(result.effectiveDate)} />
            <Row
              label="Invoice"
              value={
                result.invoice
                  ? formatCurrency(result.invoice.amount, result.invoice.currency)
                  : 'None'
              }
            />
          </div>
        )}
        <DialogFooter>
          <Button type="button" variant="outline" onClick={onClose}>
            Cancel
          </Button>
          <Button
            type="button"
            variant="primary"
            disabled={!result || end.isPending}
            onClick={() => end.mutate({ subscriptionId })}
          >
            {title}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}

const Row = ({ label, value }: { label: string; value: string }) => (
  <div className="flex items-center justify-between gap-3">
    <span className="text-muted-foreground">{label}</span>
    <span className="font-medium">{value}</span>
  </div>
)
