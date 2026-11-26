import { skipToken } from '@connectrpc/connect-query'
import {
  Badge,
  Form,
  InputFormField,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  cn,
} from '@md/ui'
import { useState } from 'react'
import { z } from 'zod'

import { type CheckAmounts, type Target } from '@/features/entitlements/explain/checkRequest'
import { displayValue } from '@/features/entitlements/explain/display'
import { isAppCounted, isLimit, isWarningReason, reasonLabel } from '@/features/entitlements/model'
import { useResourceTypes } from '@/features/resources/hooks'
import { useDebounceValue } from '@/hooks/useDebounce'
import { useZodForm } from '@/hooks/useZodForm'
import { useQuery } from '@/lib/connectrpc'
import { getCustomerById } from '@/rpc/api/customers/v1/customers-CustomersService_connectquery'
import { listFeatures } from '@/rpc/api/entitlements/v1/entitlements-EntitlementsService_connectquery'
import { type Feature, FeatureStatus } from '@/rpc/api/entitlements/v1/models_pb'

import type { CheckEntitlementResponse } from '@/rpc/api/entitlements/v1/entitlements_pb'

/** The customer, feature and target a check asks about; `customer` and `feature` fix them. */
export function useCheckSubject({
  customer: fixed,
  feature: preset,
  target: initial,
}: {
  customer?: { id: string; name: string }
  feature?: Feature
  target?: Target
}) {
  const [target, setTarget] = useState<Target>(initial ?? { kind: 'customer' })
  const [featureId, setFeatureId] = useState(preset?.id)
  const [pickedId, setPickedId] = useState<string>()
  const picked = useQuery(getCustomerById, !fixed && pickedId ? { id: pickedId } : skipToken)
  const featuresQuery = useQuery(
    listFeatures,
    preset ? skipToken : { pagination: { page: 0, perPage: 200 }, statuses: [FeatureStatus.ACTIVE] }
  )
  const features = featuresQuery.data?.features ?? []
  return {
    customerId: fixed?.id ?? pickedId,
    customerName: fixed?.name ?? picked.data?.customer?.name ?? '',
    pickedId,
    pickCustomer: (id: string) => {
      setPickedId(id || undefined)
      setTarget({ kind: 'customer' })
    },
    features,
    featureId,
    setFeatureId,
    feature: preset ?? features.find(f => f.id === featureId),
    target,
    setTarget,
  }
}

export const FeatureSelect = ({
  value,
  onChange,
  features,
  className,
}: {
  value: string | undefined
  onChange: (id: string) => void
  features: Feature[]
  className?: string
}) => (
  <Select value={value ?? ''} onValueChange={onChange}>
    <SelectTrigger className={className} aria-label="Feature">
      <SelectValue placeholder="Feature" />
    </SelectTrigger>
    <SelectContent>
      {features.map(f => (
        <SelectItem key={f.id} value={f.id}>
          {f.name}
        </SelectItem>
      ))}
    </SelectContent>
  </Select>
)

const amount = z
  .string()
  .trim()
  .refine(v => v === '' || /^\d+(\.\d+)?$/.test(v), 'A number, 0 or more')

const amountsSchema = z.object({ requested: amount, currentCount: amount })

/** The requested units, and the count of a limit counted by the app; unset while invalid. */
export function useCheckAmounts(feature: Feature | undefined) {
  const methods = useZodForm({
    schema: amountsSchema,
    defaultValues: { requested: '', currentCount: '' },
    mode: 'onChange',
  })
  const withRequested = isLimit(feature)
  const withCount = isAppCounted(feature)
  const requested = useDebounceValue(methods.watch('requested'), 300)
  const currentCount = useDebounceValue(methods.watch('currentCount'), 300)
  const parsed = amountsSchema.safeParse({
    requested: withRequested ? requested : '',
    currentCount: withCount ? currentCount : '',
  })
  const amounts: CheckAmounts | undefined = parsed.success
    ? { requested: parsed.data.requested || undefined, currentCount: parsed.data.currentCount || undefined }
    : undefined
  return { methods, amounts, withRequested, withCount }
}

export const CheckAmountsFields = ({
  methods,
  withCount,
  className,
  inputClassName,
}: {
  methods: ReturnType<typeof useCheckAmounts>['methods']
  withCount: boolean
  className?: string
  inputClassName?: string
}) => (
  <Form {...methods}>
    <form onSubmit={e => e.preventDefault()} className={cn('grid grid-cols-2 gap-3', className)}>
      <InputFormField
        control={methods.control}
        name="requested"
        label="Requested"
        inputMode="decimal"
        placeholder="1"
        className={inputClassName}
      />
      {withCount && (
        <InputFormField
          control={methods.control}
          name="currentCount"
          label="Current count"
          inputMode="decimal"
          placeholder="Stored"
          className={inputClassName}
        />
      )}
    </form>
  </Form>
)

/** Allowed or the denial's reason, the overage and near-limit flags, and the value. */
export const CheckOutcome = ({ data, feature }: { data: CheckEntitlementResponse; feature: Feature }) => {
  const { byId: types } = useResourceTypes()
  const ownType = feature.levelResourceTypeId ? types.get(feature.levelResourceTypeId) : undefined
  return (
    <div className="flex flex-wrap items-center gap-2">
      <Badge
        variant={data.allowed ? 'success' : isWarningReason(data.reason) ? 'warning' : 'destructive'}
        size="sm"
      >
        {data.allowed ? 'Allowed' : (reasonLabel(data.reason, ownType) ?? 'Denied')}
      </Badge>
      {data.allowed && data.inOverage && (
        <Badge variant="warning" size="sm">
          In overage
        </Badge>
      )}
      {data.allowed && data.nearLimit && !data.inOverage && (
        <Badge variant="secondary" size="sm">
          Near limit
        </Badge>
      )}
      <span className="text-sm font-medium tabular-nums">{displayValue(data.value, feature)}</span>
    </div>
  )
}
