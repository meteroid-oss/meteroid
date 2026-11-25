import { useMutation } from '@connectrpc/connect-query'
import {
  Button,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Skeleton,
} from '@md/ui'
import { toast } from 'sonner'

import { Gate } from '@/components/Gate'
import { useQuery } from '@/lib/connectrpc'
import { rpcErrorMessage } from '@/lib/rpcError'
import { QuantityDecrease, QuantityIncrease } from '@/rpc/api/quantities/v1/models_pb'
import {
  listManagedQuantities,
  updateManagedQuantity,
} from '@/rpc/api/quantities/v1/quantities-ManagedQuantitiesService_connectquery'

const INCREASES: [QuantityIncrease, string][] = [
  [QuantityIncrease.IMMEDIATELY, 'At once'],
  [QuantityIncrease.NEXT_PERIOD, 'From the next period'],
]

const DECREASES: [QuantityDecrease, string][] = [
  [QuantityDecrease.AT_PERIOD_END, 'At the period end'],
  [QuantityDecrease.IMMEDIATELY, 'At once, unused time given back'],
]

const labelOf = <T,>(options: [T, string][], value: T) =>
  options.find(([v]) => v === value)?.[1] ?? ''

/**
 * When a seat change takes effect. The rules belong to the managed quantity the seat product
 * counts (its code is the product's unit), so every product counting that unit shares them.
 */
export const ProductSeatsSection = ({ unitName }: { unitName: string }) => {
  const quantitiesQuery = useQuery(listManagedQuantities, {})
  const quantity = quantitiesQuery.data?.quantities.find(q => q.code === unitName)

  const update = useMutation(updateManagedQuantity, {
    onSuccess: () => {
      quantitiesQuery.refetch()
      toast.success('Seat rules updated')
    },
    onError: error => toast.error(rpcErrorMessage(error, 'The seat rules could not be updated')),
  })

  if (quantitiesQuery.isLoading) {
    return <Skeleton className="h-16 w-full" />
  }

  // Without the quantities, the default rules below could be shown in place of the real ones.
  if (quantitiesQuery.isError) {
    return (
      <div className="flex items-center gap-3" role="alert">
        <p className="text-sm text-destructive">The seat rules could not be loaded.</p>
        <Button variant="secondary" size="sm" onClick={() => void quantitiesQuery.refetch()}>
          Retry
        </Button>
      </div>
    )
  }

  if (!quantity) {
    return (
      <p className="text-sm text-muted-foreground">
        Added seats apply at once, removed seats at the period end.
      </p>
    )
  }

  return (
    <div className="flex flex-col gap-3">
      <SeatRule
        label="Adding seats"
        value={quantity.onIncrease}
        options={INCREASES}
        disabled={update.isPending}
        onChange={onIncrease => update.mutate({ id: quantity.id, onIncrease })}
      />
      <SeatRule
        label="Removing seats"
        value={quantity.onDecrease}
        options={DECREASES}
        disabled={update.isPending}
        onChange={onDecrease => update.mutate({ id: quantity.id, onDecrease })}
      />
      <p className="text-xs text-muted-foreground">
        Shared by every product that counts{' '}
        <span className="font-mono">{quantity.code}</span>.
      </p>
    </div>
  )
}

function SeatRule<T extends number>({
  label,
  value,
  options,
  disabled,
  onChange,
}: {
  label: string
  value: T
  options: [T, string][]
  disabled: boolean
  onChange: (value: T) => void
}) {
  return (
    <div className="flex items-center gap-4">
      <span className="w-28 shrink-0 text-sm text-muted-foreground">{label}</span>
      <Gate perm="catalog:manage" fallback={<span className="text-sm">{labelOf(options, value)}</span>}>
        <Select
          value={String(value)}
          disabled={disabled}
          onValueChange={next => onChange(Number(next) as T)}
        >
          <SelectTrigger className="h-8 w-[240px] text-sm">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {options.map(([option, optionLabel]) => (
              <SelectItem key={option} value={String(option)}>
                {optionLabel}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </Gate>
    </div>
  )
}
