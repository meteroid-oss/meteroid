import { createConnectQueryKey } from '@connectrpc/connect-query'
import {
  Button,
  Skeleton,
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@md/ui'
import { keepPreviousData, useQueryClient } from '@tanstack/react-query'
import { Pencil, PlusIcon } from 'lucide-react'
import { useState } from 'react'

import { Gate } from '@/components/Gate'
import { FeatureCreateSheet } from '@/features/entitlements/features/FeatureCreateSheet'
import { FeatureStatusBadge } from '@/features/entitlements/features/FeatureStatusBadge'
import { RowSheet } from '@/features/entitlements/rows/RowSheet'
import { GrantValue } from '@/features/entitlements/rows/grantDisplay'
import { useQuery } from '@/lib/connectrpc'
import { listFeatures } from '@/rpc/api/entitlements/v1/entitlements-EntitlementsService_connectquery'
import { type Feature, FeatureStatus } from '@/rpc/api/entitlements/v1/models_pb'

/** A product feature and its default, editable through the feature's default grant. */
const ProductFeatureRow = ({ feature }: { feature: Feature }) => {
  const [editing, setEditing] = useState(false)
  return (
    <TableRow>
      <TableCell className="py-2.5 pl-3 align-top">
        <div className="flex flex-col gap-0.5">
          <div className="flex flex-wrap items-center gap-2">
            <span className="text-sm font-medium">{feature.name}</span>
            {feature.status === FeatureStatus.DISABLED && <FeatureStatusBadge status={feature.status} />}
          </div>
          <span className="font-mono text-xs text-muted-foreground">{feature.code}</span>
        </div>
      </TableCell>
      <TableCell className="py-2.5 align-top">
        {feature.defaults.length ? (
          <div className="flex flex-col gap-2">
            {feature.defaults.map(row => (
              <GrantValue key={row.id} row={row} feature={feature} source="default" />
            ))}
          </div>
        ) : (
          <span className="text-sm text-muted-foreground">—</span>
        )}
      </TableCell>
      <TableCell className="w-px py-1.5 align-top">
        <Gate perm="catalog:manage">
          <Button
            type="button"
            variant="ghost"
            size="icon"
            className="h-8 w-8"
            aria-label="Edit default"
            onClick={() => setEditing(true)}
          >
            <Pencil size={14} />
          </Button>
        </Gate>
        {editing && (
          <RowSheet
            owner={{ case: 'featureId', value: feature.id }}
            feature={feature}
            existing={feature.defaults[0]}
            onClose={() => setEditing(false)}
          />
        )}
      </TableCell>
    </TableRow>
  )
}

interface Props {
  productId: string
}

const PAGE_SIZE = 100

export const ProductEntitlementsSection = ({ productId }: Props) => {
  const queryClient = useQueryClient()
  const [addOpen, setAddOpen] = useState(false)
  const [shown, setShown] = useState(PAGE_SIZE)

  const featuresQuery = useQuery(listFeatures, {
    productId,
    pagination: { page: 0, perPage: shown },
    statuses: [FeatureStatus.ACTIVE, FeatureStatus.DISABLED],
  }, { placeholderData: keepPreviousData })
  const features = featuresQuery.data?.features ?? []
  const total = featuresQuery.data?.paginationMeta?.totalItems ?? 0

  const invalidateFeatures = () =>
    queryClient.invalidateQueries({ queryKey: createConnectQueryKey({
      schema: listFeatures.parent,
      cardinality: undefined
    }) })

  return (
    <section className="flex flex-col gap-3">
      <div className="flex items-center justify-between gap-3">
        <h3 className="text-sm font-semibold">Entitlements</h3>
        <Gate perm="catalog:manage">
          <Button type="button" size="sm" variant="outline" hasIcon onClick={() => setAddOpen(true)}>
            <PlusIcon size={14} /> Add feature
          </Button>
        </Gate>
      </div>

      {featuresQuery.isLoading ? (
        <Skeleton className="h-24 w-full" />
      ) : features.length === 0 ? (
        <p className="rounded-lg border border-border px-4 py-6 text-center text-sm text-muted-foreground">
          No features
        </p>
      ) : (
        <div className="overflow-hidden rounded-lg border border-border">
          <Table>
            <TableHeader className="border-t-0">
              <TableRow>
                <TableHead className="pl-3">Feature</TableHead>
                <TableHead>Default</TableHead>
                <TableHead className="w-px">
                  <span className="sr-only">Actions</span>
                </TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {features.map(f => (
                <ProductFeatureRow key={f.id} feature={f} />
              ))}
            </TableBody>
          </Table>
        </div>
      )}
      {total > features.length && (
        <div className="flex justify-center">
          <Button
            type="button"
            size="sm"
            variant="ghost"
            disabled={featuresQuery.isFetching}
            onClick={() => setShown(n => n + PAGE_SIZE)}
          >
            Show more ({total - features.length} left)
          </Button>
        </div>
      )}

      {addOpen && (
        <FeatureCreateSheet
          initialProductId={productId}
          onClose={() => {
            setAddOpen(false)
            invalidateFeatures()
          }}
        />
      )}
    </section>
  )
}
