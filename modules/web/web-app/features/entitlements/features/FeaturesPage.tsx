import { PlusIcon } from '@md/icons'
import {
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuTrigger,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@md/ui'
import { SortingState } from '@tanstack/react-table'
import { Button } from '@ui/components'
import { ChevronDown } from 'lucide-react'
import { useState } from 'react'
import { useNavigate } from 'react-router-dom'

import { Gate } from '@/components/Gate'
import { ListPageLayout, ListToolbarSearch, RefreshButton } from '@/components/layouts'
import { FeaturesTable } from '@/features/entitlements/features/FeaturesTable'
import { useDebounceValue } from '@/hooks/useDebounce'
import { useQueryState } from '@/hooks/useQueryState'
import { useListPagination, useResetPageOnChange } from '@/hooks/useTablePreferences'
import { useQuery } from '@/lib/connectrpc'
import { listFeatures } from '@/rpc/api/entitlements/v1/entitlements-EntitlementsService_connectquery'
import { FeatureStatus } from '@/rpc/api/entitlements/v1/models_pb'
import { listProducts } from '@/rpc/api/products/v1/products-ProductsService_connectquery'

import { FeatureActionDialog, PendingFeatureAction } from './statusActions'

const ALL_STATUSES: FeatureStatus[] = [FeatureStatus.ACTIVE, FeatureStatus.DISABLED, FeatureStatus.ARCHIVED]
const DEFAULT_STATUSES: FeatureStatus[] = [FeatureStatus.ACTIVE, FeatureStatus.DISABLED]

function statusLabel(s: FeatureStatus): string {
  switch (s) {
    case FeatureStatus.ACTIVE:
      return 'Active'
    case FeatureStatus.DISABLED:
      return 'Disabled'
    case FeatureStatus.ARCHIVED:
      return 'Archived'
    default:
      return String(s)
  }
}

function selectionLabel(selection: Set<FeatureStatus>): string {
  if (selection.size === ALL_STATUSES.length) return 'All statuses'
  const shown = selection.size === 0 ? DEFAULT_STATUSES : Array.from(selection)
  return shown.map(statusLabel).join(', ')
}

export const FeaturesPage = () => {
  const navigate = useNavigate()

  const [search, setSearch] = useQueryState<string | undefined>('q', undefined)
  const debouncedSearch = useDebounceValue(search, 400)
  const [statusSelection, setStatusSelection] = useState<Set<FeatureStatus>>(
    () => new Set(DEFAULT_STATUSES)
  )
  const [productFilter, setProductFilter] = useQueryState<string | undefined>('product', undefined)
  const [pagination, setPagination] = useListPagination()
  const [sorting, setSorting] = useState<SortingState>([])
  const [pendingAction, setPendingAction] = useState<PendingFeatureAction | null>(null)

  useResetPageOnChange(setPagination, [debouncedSearch, statusSelection, productFilter])

  const productsQuery = useQuery(listProducts, { pagination: { page: 0, perPage: 200 } })
  const products = productsQuery.data?.products ?? []

  const query = useQuery(listFeatures, {
    pagination: { page: pagination.pageIndex, perPage: pagination.pageSize },
    statuses: Array.from(statusSelection),
    search: debouncedSearch || undefined,
    productId: productFilter || undefined,
  })

  const toggleStatus = (s: FeatureStatus) => {
    setStatusSelection(prev => {
      const next = new Set(prev)
      if (next.has(s)) {
        next.delete(s)
      } else {
        next.add(s)
      }
      return next
    })
  }

  return (
    <ListPageLayout
      title="Features"
      count={query.data?.paginationMeta?.totalItems}
      actions={
        <Gate perm="catalog:manage">
          <Button variant="primary" size="sm" hasIcon onClick={() => navigate('create')}>
            <PlusIcon className="w-4 h-4" /> New feature
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
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button variant="outline" size="sm" className="h-8 text-sm gap-1">
                {selectionLabel(statusSelection)}
                <ChevronDown className="w-4 h-4" />
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="start">
              {ALL_STATUSES.map(s => (
                <DropdownMenuCheckboxItem
                  key={s}
                  checked={statusSelection.has(s)}
                  onCheckedChange={() => toggleStatus(s)}
                >
                  {statusLabel(s)}
                </DropdownMenuCheckboxItem>
              ))}
            </DropdownMenuContent>
          </DropdownMenu>
          <Select
            value={productFilter ?? '__all__'}
            onValueChange={v => setProductFilter(v === '__all__' ? undefined : v)}
          >
            <SelectTrigger className="w-40 h-8 text-sm">
              <SelectValue placeholder="All products" />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="__all__">All products</SelectItem>
              {products.map(p => (
                <SelectItem key={p.id} value={p.id}>
                  {p.name}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </>
      }
      toolbarTrailing={<RefreshButton isLoading={query.isLoading} onClick={() => query.refetch()} />}
    >
      <FeaturesTable
        query={query}
        pagination={pagination}
        setPagination={setPagination}
        sorting={sorting}
        onSortingChange={setSorting}
        onAction={(feature, target) => setPendingAction({ feature, target })}
      />

      <FeatureActionDialog pending={pendingAction} onClose={() => setPendingAction(null)} />
    </ListPageLayout>
  )
}
