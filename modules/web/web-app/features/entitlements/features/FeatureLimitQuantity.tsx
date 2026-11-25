import { useMutation } from '@connectrpc/connect-query'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@md/ui'
import { toast } from 'sonner'

import { Gate } from '@/components/Gate'
import { singular } from '@/features/plans/charges/summary'
import { useQuery } from '@/lib/connectrpc'
import { rpcErrorMessage } from '@/lib/rpcError'
import {
  listManagedQuantities,
  setFeatureLimitQuantity,
} from '@/rpc/api/quantities/v1/quantities-ManagedQuantitiesService_connectquery'

// A quantity's name is its plural unit ("Seats"): "Per seat".
const perUnit = (name: string) => `Per ${singular(name)}`

// Radix Select forbids empty-string item values.
const PER_SUBSCRIPTION = '__per_subscription__'

/** Whether a metered feature's limit counts per subscription or per unit of a seat quantity. */
export const FeatureLimitQuantity = ({
  featureId,
  readOnly,
}: {
  featureId: string
  readOnly?: boolean
}) => {
  const quantitiesQuery = useQuery(listManagedQuantities, {})
  const quantities = quantitiesQuery.data?.quantities ?? []
  const current = quantities.find(q => q.perUnitFeatureIds.includes(featureId))
  const selectable = quantities.filter(q => !q.archived || q.id === current?.id)

  const setLimit = useMutation(setFeatureLimitQuantity, {
    onSuccess: () => {
      quantitiesQuery.refetch()
      toast.success('Limit counting updated')
    },
    onError: error => toast.error(rpcErrorMessage(error, 'The limit could not be changed')),
  })

  if (quantitiesQuery.isLoading || selectable.length === 0) return null

  const label = current ? perUnit(current.name) : 'Per subscription'

  return (
    <section>
      <div className="text-xs font-medium text-muted-foreground uppercase tracking-wide mb-1">
        Count the limit
      </div>
      {readOnly ? (
        <p className="text-sm">{label}</p>
      ) : (
        <Gate perm="catalog:manage" fallback={<p className="text-sm">{label}</p>}>
          <Select
            value={current?.id ?? PER_SUBSCRIPTION}
            disabled={setLimit.isPending}
            onValueChange={value =>
              setLimit.mutate({
                featureId,
                managedQuantityId: value === PER_SUBSCRIPTION ? undefined : value,
              })
            }
          >
            <SelectTrigger className="h-8 w-[240px] text-sm">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value={PER_SUBSCRIPTION}>Per subscription</SelectItem>
              {selectable.map(quantity => (
                <SelectItem key={quantity.id} value={quantity.id}>
                  {perUnit(quantity.name)}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </Gate>
      )}
      <p className="mt-1 text-xs text-muted-foreground">
        Per unit, the limit multiplies by the units held: 100 per seat gives a subscription with 5
        seats 500.
      </p>
    </section>
  )
}
