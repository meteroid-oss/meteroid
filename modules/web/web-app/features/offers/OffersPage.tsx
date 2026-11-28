import { Badge, Button, Tabs, TabsList, TabsTrigger } from '@md/ui'
import { ColumnDef } from '@tanstack/react-table'
import { useMemo } from 'react'
import { useLocation, useNavigate, useParams } from 'react-router'

import { Gate } from '@/components/Gate'
import { AddIcon } from '@/components/icons/AppIcons'
import { ListPageLayout, RefreshButton } from '@/components/layouts'
import { StandardTable } from '@/components/table/StandardTable'
import { Slot, hasSlot } from '@/edition'
import { PlanLockChip } from '@/features/settings/components/PlanLockChip'
import { entitlementCodes, useEntitlement } from '@/hooks/useEntitlement'
import { useQueryState } from '@/hooks/useQueryState'
import { useListPagination, useResetPageOnChange } from '@/hooks/useTablePreferences'
import { useQuery } from '@/lib/connectrpc'
import { listOffers } from '@/rpc/api/offers/v1/offers-OffersService_connectquery'
import { listPlans } from '@/rpc/api/plans/v1/plans-PlansService_connectquery'

import { GrantChips, useGrantNames } from './GrantChips'
import { OffersTabs } from './OffersTabs'
import { grantsFromProto } from './grants'
import { offerChips } from './offers'

import type { Offer } from '@/rpc/api/offers/v1/offers_pb'

const NEW = 'new'

/** The catalog's offers; a module edits them (`offers.editor`), Core reads them. */
export const OffersPage = () => {
  const navigate = useNavigate()
  const { search } = useLocation()
  const { offerId } = useParams<{ offerId?: string }>()
  const [view, setView] = useQueryState<string>('view', 'active')
  const [pagination, setPagination] = useListPagination()
  useResetPageOnChange(setPagination, [view])
  const entitled = useEntitlement(entitlementCodes.SUBSCRIPTION_PHASES)
  const editable = hasSlot('offers.editor')

  const offersQuery = useQuery(listOffers, { includeArchived: view === 'all' })
  const offers = offersQuery.data?.offers ?? []
  const plans = useQuery(listPlans, {}).data?.plans ?? []
  const allGrants = useMemo(() => offers.flatMap(offer => grantsFromProto(offer.grants)), [offers])
  const names = useGrantNames(allGrants)

  const planName = (id: string) => plans.find(plan => plan.id === id)?.name

  const columns: ColumnDef<Offer>[] = [
    {
      header: 'Offer',
      cell: ({ row }) => (
        <div className="min-w-0">
          <span className="flex items-center gap-2 font-medium">
            {row.original.name}
            {row.original.archived && (
              <Badge variant="secondary" size="sm">
                Archived
              </Badge>
            )}
          </span>
          {row.original.description && (
            <span className="block max-w-xs truncate text-xs text-muted-foreground">
              {row.original.description}
            </span>
          )}
        </div>
      ),
    },
    {
      header: 'Gives',
      cell: ({ row }) => <GrantChips chips={offerChips(row.original, names)} />,
    },
    {
      header: 'Start offer of',
      cell: ({ row }) => {
        const ids = row.original.usedBy?.planIds ?? []
        return ids.length ? (
          <span>{ids.map(id => planName(id) ?? id).join(', ')}</span>
        ) : (
          <span className="text-muted-foreground">—</span>
        )
      },
    },
    {
      header: 'Coupons',
      cell: ({ row }) => {
        const coupons = row.original.usedBy?.coupons ?? []
        if (!coupons.length) return <span className="text-muted-foreground">—</span>
        return (
          <span className="flex flex-wrap gap-1">
            {coupons.map(coupon => (
              <code key={coupon.id} className="font-mono text-xs">
                {coupon.code || coupon.id}
              </code>
            ))}
          </span>
        )
      },
    },
    {
      header: 'Used',
      cell: ({ row }) => {
        const count = Number(row.original.usedBy?.applications ?? 0n)
        return count ? (
          <span className="tabular-nums">
            {count} {count === 1 ? 'subscription' : 'subscriptions'}
          </span>
        ) : (
          <span className="text-muted-foreground">—</span>
        )
      },
    },
  ]

  const page = offers.slice(
    pagination.pageIndex * pagination.pageSize,
    (pagination.pageIndex + 1) * pagination.pageSize
  )
  const closeEditor = () => navigate({ pathname: '.', search })

  return (
    <>
      <ListPageLayout
        title="Offers"
        tabs={<OffersTabs active="offers" />}
        actions={
          editable ? (
            <Gate perm="catalog:manage">
              <span className="flex items-center gap-2">
                {!entitled && <PlanLockChip />}
                <Button
                  variant="primary"
                  size="sm"
                  hasIcon
                  disabled={!entitled}
                  onClick={() => navigate({ pathname: NEW, search })}
                >
                  <AddIcon /> New offer
                </Button>
              </span>
            </Gate>
          ) : (
            <PlanLockChip />
          )
        }
        footer={
          <Tabs value={view} onValueChange={setView}>
            <TabsList variant="ghost">
              <TabsTrigger value="active">Active</TabsTrigger>
              <TabsTrigger value="all">All</TabsTrigger>
            </TabsList>
          </Tabs>
        }
        toolbarTrailing={
          <RefreshButton isLoading={offersQuery.isLoading} onClick={() => offersQuery.refetch()} />
        }
      >
        <StandardTable
          tableKey="offers"
          columns={columns}
          data={page}
          pagination={pagination}
          setPagination={setPagination}
          totalCount={offers.length}
          isLoading={offersQuery.isLoading}
          emptyMessage="No offers"
          rowLink={editable ? row => row.original.id : undefined}
          rowClassName={row => (row.original.id === offerId ? 'bg-accent/50' : '')}
        />
      </ListPageLayout>
      {editable && offerId && (
        <Slot
          name="offers.editor"
          offerId={offerId === NEW ? undefined : offerId}
          onClose={closeEditor}
        />
      )}
    </>
  )
}
