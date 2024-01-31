import { Table } from '@md/ui'
import { ColumnDef, OnChangeFn, PaginationState, Row, flexRender } from '@tanstack/react-table'
import { ReactNode } from 'react'

import { CustomTable } from '@/components/table/CustomTable'

interface StandardTableProps<A> {
  columns: ColumnDef<A>[]
  data: A[] | undefined
  sortable?: boolean
  pagination: PaginationState
  setPagination: OnChangeFn<PaginationState>
  totalCount: number
  emptyMessage?: string | ReactNode
  isLoading?: boolean
}
export const StandardTable = <A extends object>({
  columns,
  data,
  sortable,
  pagination,
  setPagination,
  totalCount,
  emptyMessage = 'No data to display',
  isLoading,
}: StandardTableProps<A>) => {
  return (
    <CustomTable
      columns={columns}
      data={data}
      sortable={sortable}
      pagination={pagination}
      setPagination={setPagination}
      totalCount={totalCount}
      emptyMessage={emptyMessage}
      rowRenderer={standardRowRenderer}
      isLoading={isLoading}
    />
  )
}

const standardRowRenderer = <A extends object>(row: Row<A>) => {
  return (
    <Table.tr key={row.id}>
      {row.getVisibleCells().map(cell => {
        return (
          <Table.td key={cell.id}>
            {flexRender(cell.column.columnDef.cell, cell.getContext())}
          </Table.td>
        )
      })}
    </Table.tr>
  )
}
