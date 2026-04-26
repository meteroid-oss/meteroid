import { useMutation } from '@connectrpc/connect-query'
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
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
import { useQueryClient } from '@tanstack/react-query'
import { PaginationState, SortingState } from '@tanstack/react-table'
import { Button } from '@ui/components'
import { ChevronDown } from 'lucide-react'
import { useEffect, useState } from 'react'

import { FeaturesHeader } from '@/features/entitlements/features/FeaturesHeader'
import { FeaturesTable } from '@/features/entitlements/features/FeaturesTable'
import { useDebounceValue } from '@/hooks/useDebounce'
import { useQueryState } from '@/hooks/useQueryState'
import { useQuery } from '@/lib/connectrpc'
import {
  listFeatures,
  setFeatureStatus,
} from '@/rpc/api/entitlements/v1/entitlements-EntitlementsService_connectquery'
import { Feature, FeatureStatus } from '@/rpc/api/entitlements/v1/models_pb'
import { listProducts } from '@/rpc/api/products/v1/products-ProductsService_connectquery'

// Archive is hidden from the catalog UI — keep it out of the filter set too.
const ALL_STATUSES: FeatureStatus[] = [FeatureStatus.ACTIVE, FeatureStatus.DISABLED]

function statusLabel(s: FeatureStatus): string {
  switch (s) {
    case FeatureStatus.ACTIVE:
      return 'Active'
    case FeatureStatus.DISABLED:
      return 'Disabled'
    default:
      return String(s)
  }
}

function selectionLabel(selection: Set<FeatureStatus>): string {
  if (selection.size === 0 || selection.size === ALL_STATUSES.length) return 'All statuses'
  return Array.from(selection).map(statusLabel).join(', ')
}

type StatusAction = { feature: Feature; target: FeatureStatus }

function actionCopy(target: FeatureStatus): { title: string; description: string; cta: string } {
  if (target === FeatureStatus.ACTIVE)
    return {
      title: 'Re-activate feature?',
      description: 'Customers regain access to this feature. All entitlement settings come back as they were.',
      cta: 'Re-activate',
    }
  return {
    title: 'Disable feature?',
    description: "Customers will lose access while it's off. Settings stay saved — turning it back on restores everything.",
    cta: 'Disable',
  }
}

export const FeaturesPage = () => {
  const queryClient = useQueryClient()

  const [search] = useQueryState<string | undefined>('q', undefined)
  const debouncedSearch = useDebounceValue(search, 400)
  const [statusSelection, setStatusSelection] = useState<Set<FeatureStatus>>(
    () => new Set([FeatureStatus.ACTIVE, FeatureStatus.DISABLED])
  )
  const [productFilter, setProductFilter] = useQueryState<string | undefined>('product', undefined)
  const [pagination, setPagination] = useState<PaginationState>({ pageIndex: 0, pageSize: 20 })
  const [sorting, setSorting] = useState<SortingState>([])
  const [pendingAction, setPendingAction] = useState<StatusAction | null>(null)

  useEffect(() => {
    setPagination(prev => ({ ...prev, pageIndex: 0 }))
  }, [debouncedSearch, statusSelection, productFilter])

  const productsQuery = useQuery(listProducts, { pagination: { page: 0, perPage: 200 } })
  const products = productsQuery.data?.products ?? []

  const query = useQuery(listFeatures, {
    pagination: { page: pagination.pageIndex, perPage: pagination.pageSize },
    statuses: Array.from(statusSelection),
    search: debouncedSearch || undefined,
    productId: productFilter || undefined,
  })

  const setStatusMutation = useMutation(setFeatureStatus, {
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: [listFeatures.service.typeName] })
      setPendingAction(null)
    },
  })

  const copy = pendingAction ? actionCopy(pendingAction.target) : null

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
    <>
      <FeaturesHeader
        count={query.data?.paginationMeta?.totalItems}
        isLoading={query.isLoading}
        refetch={() => query.refetch()}
      >
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
      </FeaturesHeader>
      <FeaturesTable
        query={query}
        pagination={pagination}
        setPagination={setPagination}
        sorting={sorting}
        onSortingChange={setSorting}
        onStatusAction={(feature, target) => setPendingAction({ feature, target })}
      />

      <AlertDialog open={!!pendingAction} onOpenChange={open => !open && setPendingAction(null)}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{copy?.title}</AlertDialogTitle>
            <AlertDialogDescription>{copy?.description}</AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              onClick={() => {
                if (!pendingAction) return
                setStatusMutation.mutate({
                  id: pendingAction.feature.id,
                  status: pendingAction.target,
                })
              }}
            >
              {copy?.cta}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  )
}
