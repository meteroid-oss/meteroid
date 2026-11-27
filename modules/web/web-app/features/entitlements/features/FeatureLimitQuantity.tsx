import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { InfoIcon } from 'lucide-react'
import { type ComponentType, type ReactNode } from 'react'
import { toast } from 'sonner'

import { Gate } from '@/components/Gate'
import { singular } from '@/features/plans/charges/summary'
import { useQuery } from '@/lib/connectrpc'
import { rpcErrorMessage } from '@/lib/rpcError'
import { listEntitlementsByEntity } from '@/rpc/api/entitlements/v1/entitlements-EntitlementsService_connectquery'
import {
  listManagedQuantities,
  setFeatureLimitQuantity,
} from '@/rpc/api/quantities/v1/quantities-ManagedQuantitiesService_connectquery'

// A quantity's name is its plural unit ("Seats"): "Per seat".
const perUnit = (name: string) => `Per ${singular(name)}`

// Radix Select forbids empty-string item values.
const PER_SUBSCRIPTION = '__per_subscription__'

/** Whether a limit feature counts per subscription or per unit of a seat quantity. */
export const FeatureLimitQuantity = ({
  featureId,
  readOnly,
  Row,
}: {
  featureId: string
  readOnly?: boolean
  Row: ComponentType<{ label: ReactNode; children: ReactNode }>
}) => {
  const queryClient = useQueryClient()
  const quantitiesQuery = useQuery(listManagedQuantities, {})
  const quantities = quantitiesQuery.data?.quantities ?? []
  const current = quantities.find(q => q.perUnitFeatureIds.includes(featureId))
  const selectable = quantities.filter(q => !q.archived || q.id === current?.id)

  const setLimit = useMutation(setFeatureLimitQuantity, {
    onSuccess: () => {
      quantitiesQuery.refetch()
      // Resolved values scale by the units held (R-133).
      queryClient.invalidateQueries({
        queryKey: createConnectQueryKey({ schema: listEntitlementsByEntity.parent, cardinality: undefined }),
      })
      toast.success('Limit counting updated')
    },
    onError: error => toast.error(rpcErrorMessage(error, 'The limit could not be changed')),
  })

  if (quantitiesQuery.isLoading || selectable.length === 0) return null

  const label = current ? perUnit(current.name) : 'Per subscription'

  return (
    <Row
      label={
        <span className="inline-flex items-center gap-1">
          Limit counted
          <Tooltip>
            <TooltipTrigger asChild>
              <InfoIcon className="h-3.5 w-3.5 cursor-help text-muted-foreground" />
            </TooltipTrigger>
            <TooltipContent multiline className="max-w-80">
              Per seat, the limit multiplies by the seats held
            </TooltipContent>
          </Tooltip>
        </span>
      }
    >
      {readOnly ? (
        label
      ) : (
        <Gate perm="catalog:manage" fallback={label}>
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
    </Row>
  )
}
