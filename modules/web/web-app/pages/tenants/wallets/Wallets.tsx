import { useEffect, useMemo, useState } from 'react'

import { ListPageLayout, ListToolbarSearch, RefreshButton } from '@/components/layouts'
import { StandardTable } from '@/components/table/StandardTable'
import { Slot, slotItems } from '@/edition'
import { WalletStatusBadge } from '@/features/credit/WalletStatusBadge'
import { formatWalletAmount, walletTitle } from '@/features/credit/format'
import {
  liveUpdatingDescription,
  liveUpdatingLabel,
  walletHealthDescription,
} from '@/features/credit/walletStatus'
import { useBasePath } from '@/hooks/useBasePath'
import { useDebounceValue } from '@/hooks/useDebounce'
import { useQueryState } from '@/hooks/useQueryState'
import { useQuery } from '@/lib/connectrpc'
import { WalletStatus } from '@/rpc/api/wallets/v1/models_pb'
import { listWalletSummaries } from '@/rpc/api/wallets/v1/wallets-WalletsService_connectquery'

import { WalletStatusFilter, parseWalletFilter, walletFilterRequest } from './WalletStatusFilter'

import type { WalletFilter } from './WalletStatusFilter'
import type { WalletSummary } from '@/rpc/api/wallets/v1/models_pb'
import type { ColumnDef, PaginationState } from '@tanstack/react-table'

const filterSerde = {
  serialize: (value: WalletFilter | undefined) => value ?? '',
  deserialize: parseWalletFilter,
}

export const Wallets = () => {
  const basePath = useBasePath()
  const [text, setText] = useState('')
  const search = useDebounceValue(text, 400)
  const [filter, setFilter] = useQueryState<WalletFilter | undefined>(
    'status',
    undefined,
    filterSerde
  )
  const [pagination, setPagination] = useState<PaginationState>({ pageIndex: 0, pageSize: 20 })

  useEffect(() => {
    setPagination(prev => ({ ...prev, pageIndex: 0 }))
  }, [search, filter])

  const query = useQuery(listWalletSummaries, {
    search: search || undefined,
    ...walletFilterRequest(filter),
    pagination: { page: pagination.pageIndex, perPage: pagination.pageSize },
  })
  const data = query.data?.wallets ?? []
  const count = query.data?.paginationMeta?.totalItems ?? 0

  const columns = useMemo<ColumnDef<WalletSummary>[]>(
    () => [
      {
        header: 'Customer',
        accessorFn: row => row.customerName,
        cell: ({ row }) => (
          <div>
            <div>{row.original.customerName}</div>
            {row.original.customerAlias && (
              <div className="text-xs text-muted-foreground">{row.original.customerAlias}</div>
            )}
          </div>
        ),
      },
      { header: 'Wallet', accessorFn: row => walletTitle(row) },
      {
        header: 'Balance',
        cell: ({ row }) =>
          row.original.live?.stale ? (
            <span className="text-sm text-muted-foreground" title={liveUpdatingDescription}>
              {liveUpdatingLabel}
            </span>
          ) : (
            formatWalletAmount(
              row.original.live?.availableAfterUsage ?? row.original.available,
              row.original
            )
          ),
      },
      {
        header: 'Status',
        cell: ({ row }) => (
          <div className="flex flex-wrap items-center gap-1.5">
            {/* The roster names every row's state, a wallet in good standing included. */}
            {row.original.status === WalletStatus.HEALTHY && !row.original.live?.stale ? (
              <span
                className="text-sm text-muted-foreground"
                title={walletHealthDescription.healthy}
              >
                Healthy
              </span>
            ) : (
              <WalletStatusBadge status={row.original.status} stale={row.original.live?.stale} />
            )}
            <Slot name="walletList.statusBadges" wallet={row.original} />
          </div>
        ),
      },
      ...slotItems('walletList.columns').map(
        (column): ColumnDef<WalletSummary> => ({
          id: column.id,
          header: column.header,
          cell: ({ row }) => <column.Cell wallet={row.original} />,
        })
      ),
    ],
    []
  )

  return (
    <ListPageLayout
      title="Wallets"
      count={count}
      toolbar={
        <>
          <ListToolbarSearch
            value={text}
            onChange={setText}
            placeholder="Search customers"
            className="max-w-xs"
          />
          <WalletStatusFilter value={filter} onChange={setFilter} />
        </>
      }
      toolbarTrailing={
        <RefreshButton isLoading={query.isLoading} onClick={() => query.refetch()} />
      }
    >
      <StandardTable
        columns={columns}
        data={data}
        pagination={pagination}
        setPagination={setPagination}
        totalCount={count}
        isLoading={query.isLoading}
        emptyMessage={filter ? 'No wallet matches this status.' : 'No customer holds a wallet yet.'}
        rowLink={row => `${basePath}/customers/${row.original.customerId}?tab=wallets`}
      />
    </ListPageLayout>
  )
}
