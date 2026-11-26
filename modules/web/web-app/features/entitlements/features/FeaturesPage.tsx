import { PlusIcon } from '@md/icons'
import {
  Button,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Tabs,
  TabsList,
  TabsTrigger,
} from '@md/ui'
import { useCallback, useState } from 'react'
import { useNavigate } from 'react-router'

import { Gate } from '@/components/Gate'
import { ListPageLayout, ListToolbarSearch, RefreshButton } from '@/components/layouts'
import { FeaturesTable } from '@/features/entitlements/features/FeaturesTable'
import { FeatureActionDialog, PendingFeatureAction } from '@/features/entitlements/features/statusActions'
import { TypeFilter } from '@/features/resources/TypeSelect'
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

const ALL_PRODUCTS = '__all__'

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

  const query = useQuery(listFeatures, {
    pagination: { page: pagination.pageIndex, perPage: pagination.pageSize },
    statuses: [...STATUS_TABS[tab].statuses],
    search: debouncedSearch || undefined,
    productId: productFilter || undefined,
    levelResourceTypeId: appliesPer,
    order: groupByProduct ? FeatureOrder.PRODUCT : FeatureOrder.UNSPECIFIED,
  })

  return (
    <ListPageLayout
      title="Features"
      count={query.data?.paginationMeta?.totalItems}
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
            <PlusIcon className="h-4 w-4" /> New feature
          </Button>
        </Gate>
      }
      toolbar={
        <>
          <ListToolbarSearch
            value={search ?? ''}
            onChange={setSearch}
            placeholder="Search features"
            className="max-w-[220px]"
          />
          <Select
            value={productFilter ?? ALL_PRODUCTS}
            onValueChange={v => setProductFilter(v === ALL_PRODUCTS ? undefined : v)}
          >
            <SelectTrigger className="h-8 w-40 text-sm">
              <SelectValue placeholder="All products" />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value={ALL_PRODUCTS}>All products</SelectItem>
              {products.map(p => (
                <SelectItem key={p.id} value={p.id}>
                  {p.name}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          <TypeFilter value={appliesPer} onChange={setAppliesPer} anyLabel="Applies per: any" />
        </>
      }
      toolbarTrailing={<RefreshButton isLoading={query.isLoading} onClick={() => query.refetch()} />}
    >
      <FeaturesTable
        query={query}
        pagination={pagination}
        setPagination={setPagination}
        onAction={onAction}
        groupByProduct={groupByProduct}
      />

      <FeatureActionDialog pending={pendingAction} onClose={() => setPendingAction(null)} />
    </ListPageLayout>
  )
}
