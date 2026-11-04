import { InputFormField } from '@md/ui'
import { Control, FieldPath, FieldValues } from 'react-hook-form'
import { z } from 'zod'

import { formatDateOnly, parseAndFormatDateTime } from '@/lib/utils/date'

import type { UsageFilter } from '@/rpc/api/subscriptions/v1/models_pb'

const MAX_BYTES = 256
const encoder = new TextEncoder()

const filterPart = z
  .string()
  .refine(v => v === v.trim(), 'Remove the leading and trailing spaces')
  .refine(v => encoder.encode(v).length <= MAX_BYTES, `At most ${MAX_BYTES} bytes`)
  .optional()

export const usageFilterFormFields = {
  usageFilterProperty: filterPart,
  usageFilterValue: filterPart,
}

export interface UsageFilterFormValues {
  usageFilterProperty?: string
  usageFilterValue?: string
}

export const refineUsageFilter = (data: UsageFilterFormValues, ctx: z.RefinementCtx) => {
  const property = data.usageFilterProperty ?? ''
  const value = data.usageFilterValue ?? ''
  if (property && !value) {
    ctx.addIssue({
      code: 'custom',
      message: 'Required with a property',
      path: ['usageFilterValue'],
    })
  }
  if (value && !property) {
    ctx.addIssue({
      code: 'custom',
      message: 'Required with a value',
      path: ['usageFilterProperty'],
    })
  }
}

export const usageFilterFromForm = (
  data: UsageFilterFormValues
): { property: string; value: string } | undefined =>
  data.usageFilterProperty && data.usageFilterValue
    ? { property: data.usageFilterProperty, value: data.usageFilterValue }
    : undefined

export const usageFilterToForm = (
  filter: Pick<UsageFilter, 'property' | 'value'> | undefined
): Required<UsageFilterFormValues> => ({
  usageFilterProperty: filter?.property ?? '',
  usageFilterValue: filter?.value ?? '',
})

export const formatUsageFilter = (filter: Pick<UsageFilter, 'property' | 'value'>) =>
  `${filter.property} = ${filter.value}`

// A filter applies from a period boundary (midnight UTC), unless a subscription ended mid-day.
export const formatUsageFilterSince = (since: string) =>
  new Date(since).getUTCHours() === 0 && new Date(since).getUTCMinutes() === 0
    ? formatDateOnly(since)
    : parseAndFormatDateTime(since)

export const USAGE_FILTER_CHANGE_WARNING =
  "The change applies to the usage not invoiced yet: from the start of this subscription's current billing period, or later if the customer's other subscriptions have already invoiced past it. Usage already invoiced keeps its attribution."

export const USAGE_FILTER_HELP =
  "Bill only the customer's events whose property has this value. If filters on different properties match the same event, the older subscription bills it. Leave empty to bill the events no other subscription's filter matches: two subscriptions without a filter both bill the same events."

export const USAGE_FILTER_NONE_HELP =
  "Bills the events no other subscription's usage filter matches. If another subscription has no filter either, it bills them too."

export const UsageFilterFields = <T extends FieldValues & UsageFilterFormValues>({
  control,
}: {
  control: Control<T>
}) => (
  <div className="space-y-2">
    <div className="grid grid-cols-2 gap-3">
      <InputFormField
        name={'usageFilterProperty' as FieldPath<T>}
        control={control}
        label="Property"
        placeholder="api_key"
        autoComplete="off"
      />
      <InputFormField
        name={'usageFilterValue' as FieldPath<T>}
        control={control}
        label="Value"
        placeholder="key-A"
        autoComplete="off"
      />
    </div>
    <p className="text-xs text-muted-foreground">{USAGE_FILTER_HELP}</p>
  </div>
)
