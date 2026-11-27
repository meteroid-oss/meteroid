import { Button, Tabs, TabsList, TabsTrigger } from '@md/ui'
import { useCallback, useState } from 'react'
import { useNavigate } from 'react-router'

import { Gate } from '@/components/Gate'
import { EmptyState } from '@/components/empty-state/EmptyState'
import { AddIcon } from '@/components/icons/AppIcons'
import { ListPageLayout, ListToolbarSearch, RefreshButton } from '@/components/layouts'
import { FeaturesTable } from '@/features/entitlements/features/FeaturesTable'
import { FeatureActionDialog, PendingFeatureAction } from '@/features/entitlements/features/statusActions'
import { FilterBar, optionField } from '@/features/filters/FilterBar'
import { useResourceTypes } from '@/features/resources/hooks'
import { useDebounceValue } from '@/hooks/useDebounce'
import { useQueryState } from '@/hooks/useQueryState'
import { useListPagination, useResetPageOnChange } from '@/hooks/useTablePreferences'
import { useQuery } from '@/lib/connectrpc'
import { listFeatures } from '@/rpc/api/entitlements/v1/entitlements-EntitlementsService_connectquery'
import { FeatureOrder } from '@/rpc/api/entitlements/v1/entitlements_pb'
import { type Feature, FeatureStatus } from '@/rpc/api/entitlements/v1/models_pb'
import { listProducts } from '@/rpc/api/products/v1/products-ProductsService_connectquery'

const STATUS_TABS = {
  current: { label: 'All', statuses: [FeatureStatus.ACTIVE, FeatureStatus.DISABLED] },
  active: { label: 'Active', statuses: [FeatureStatus.ACTIVE] },
  disabled: { label: 'Disabled', statuses: [FeatureStatus.DISABLED] },
  archived: { label: 'Archived', statuses: [FeatureStatus.ARCHIVED] },
} as const

type StatusTab = keyof typeof STATUS_TABS

const ALL_STATUSES = [FeatureStatus.ACTIVE, FeatureStatus.DISABLED, FeatureStatus.ARCHIVED]
const CUSTOMER = '__customer__'

export const FeaturesPage = () => {
  const navigate = useNavigate()

  const [search, setSearch] = useQueryState<string | undefined>('q', undefined)
  const debouncedSearch = useDebounceValue(search, 400)
  const [requestedTab, setTab] = useQueryState<string>('status', 'current')
  const tab: StatusTab = requestedTab in STATUS_TABS ? (requestedTab as StatusTab) : 'current'
  const [productFilter, setProductFilter] = useQueryState<string | undefined>('product', undefined)
  const [appliesPer, setAppliesPer] = useState<string>()
  const [pagination, setPagination] = useListPagination()
  const [pendingAction, setPendingAction] = useState<PendingFeatureAction | null>(null)

  useResetPageOnChange(setPagination, [debouncedSearch, tab, productFilter, appliesPer])
  const onAction = useCallback(
    (feature: Feature, target: FeatureStatus) => setPendingAction({ feature, target }),
    []
  )

  const groupByProduct = !productFilter

  const products = useQuery(listProducts, { pagination: { page: 0, perPage: 200 } }).data?.products ?? []
  const { all: resourceTypes, hasTypes } = useResourceTypes()

  const query = useQuery(listFeatures, {
    pagination: { page: pagination.pageIndex, perPage: pagination.pageSize },
    statuses: [...STATUS_TABS[tab].statuses],
    search: debouncedSearch || undefined,
    productId: productFilter || undefined,
    levelResourceTypeId: appliesPer,
    order: groupByProduct ? FeatureOrder.PRODUCT : FeatureOrder.UNSPECIFIED,
  })

  // No feature at all gets the empty state. The default view hides archived features, so when it
  // is empty one feature of any status is looked for first.
  const isNarrowed =
    !!debouncedSearch || !!productFilter || appliesPer !== undefined || tab !== 'current'
  const defaultViewEmpty =
    !isNarrowed &&
    pagination.pageIndex === 0 &&
    query.isSuccess &&
    (query.data?.paginationMeta?.totalItems ?? 0) === 0
  const anyFeatureQuery = useQuery(
    listFeatures,
    { pagination: { page: 0, perPage: 1 }, statuses: ALL_STATUSES },
    { enabled: defaultViewEmpty }
  )
  const isEmpty =
    defaultViewEmpty &&
    anyFeatureQuery.isSuccess &&
    (anyFeatureQuery.data?.paginationMeta?.totalItems ?? 0) === 0

  return (
    <ListPageLayout
      title="Features"
      tabs={
        <Tabs value={tab} onValueChange={setTab}>
          <TabsList variant="ghost">
            {(Object.keys(STATUS_TABS) as StatusTab[]).map(key => (
              <TabsTrigger key={key} value={key}>
                {STATUS_TABS[key].label}
              </TabsTrigger>
            ))}
          </TabsList>
        </Tabs>
      }
      actions={
        <Gate perm="catalog:manage">
          <Button variant="primary" size="sm" hasIcon onClick={() => navigate('create')}>
            <AddIcon /> New feature
          </Button>
        </Gate>
      }
      toolbar={
        <>
          <ListToolbarSearch
            value={search ?? ''}
            onChange={setSearch}
            placeholder="Search features"
          />
          <FilterBar
            local={[
              {
                field: optionField(
                  'product',
                  'Product',
                  products.map(product => ({ value: product.id, label: product.name })),
                  { multi: false }
                ),
                values: productFilter ? [productFilter] : [],
                set: ([value]) => setProductFilter(value),
              },
              ...(hasTypes
                ? [
                    {
                      field: optionField(
                        'appliesPer',
                        'Applies per',
                        [
                          { value: CUSTOMER, label: 'Customer' },
                          ...resourceTypes.map(t => ({ value: t.id, label: t.name })),
                        ],
                        { multi: false }
                      ),
                      values: appliesPer === undefined ? [] : [appliesPer || CUSTOMER],
                      set: ([value]: string[]) =>
                        setAppliesPer(value === undefined ? undefined : value === CUSTOMER ? '' : value),
                    },
                  ]
                : []),
            ]}
          />
        </>
      }
      toolbarTrailing={<RefreshButton isLoading={query.isLoading} onClick={() => query.refetch()} />}
    >
      {isEmpty ? (
        <EmptyState
          title="No features yet"
          description="Features decide what each plan gives access to, from simple switches to usage limits."
          scene="features"
          actions={
            <Gate perm="catalog:manage">
              <Button size="sm" variant="default" onClick={() => navigate('create')}>
                New feature
              </Button>
            </Gate>
          }
        />
      ) : (
        <FeaturesTable
          query={query}
          pagination={pagination}
          setPagination={setPagination}
          onAction={onAction}
          groupByProduct={groupByProduct}
        />
      )}

      <FeatureActionDialog pending={pendingAction} onClose={() => setPendingAction(null)} />
    </ListPageLayout>
  )
}
