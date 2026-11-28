import { Badge } from '@md/ui'
import { useMemo } from 'react'

import { plainValue } from '@/features/entitlements/rows/grantDisplay'
import { useFeatures } from '@/features/entitlements/rows/useFeatures'
import { useQuery } from '@/lib/connectrpc'
import { listAddOns } from '@/rpc/api/addons/v1/addons-AddOnsService_connectquery'

import type { GrantChip, GrantForm, GrantNames } from './grants'

/** Names the features and add-ons the grants refer to, as they load. */
export const useGrantNames = (grants: GrantForm[]): GrantNames => {
  const featureIds = useMemo(
    () => [...new Set(grants.flatMap(g => (g.kind === 'entitlement' ? [g.featureId] : [])))],
    [grants]
  )
  const hasAddOns = grants.some(g => g.kind === 'addOn')
  const features = useFeatures(featureIds)
  const addOns = useQuery(
    listAddOns,
    { pagination: { perPage: 100, page: 0 } },
    { enabled: hasAddOns }
  ).data?.addOns
  return {
    feature: id => features.get(id)?.name,
    addOn: id => addOns?.find(addOn => addOn.id === id)?.name,
    entitlementValue: (featureId, value) => {
      const feature = features.get(featureId)
      return feature ? plainValue(value, feature) : undefined
    },
  }
}

export const GrantChips = ({ chips }: { chips: GrantChip[] }) =>
  chips.length === 0 ? (
    <span className="text-muted-foreground">—</span>
  ) : (
    <div className="flex flex-wrap gap-1">
      {chips.map(chip => (
        <Badge key={chip.key} variant={chip.tone} size="sm">
          {chip.label}
        </Badge>
      ))}
    </div>
  )
