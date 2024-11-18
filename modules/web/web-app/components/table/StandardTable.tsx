import { TableCell, TableRow } from '@md/ui'
import { ColumnDef, OnChangeFn, PaginationState, Row, flexRender } from '@tanstack/react-table'
import { ReactNode } from 'react'
import { Link } from 'react-router-dom'

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
  rowLink?: (row: Row<A>) => string
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
  rowLink,
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
      rowRenderer={row => standardRowRenderer(row, rowLink)}
      isLoading={isLoading}
    />
  )
}

const standardRowRenderer = <A extends object>(row: Row<A>, rowLink?: (row: Row<A>) => string) => {
  const cells = row
    .getVisibleCells()
    .map(cell => (
      <TableCell key={cell.id}>
        {flexRender(cell.column.columnDef.cell, cell.getContext())}
      </TableCell>
    ))

  if (rowLink) {
    return (
      <TableRow key={row.id}>
        <Link to={rowLink(row)} style={{ display: 'contents' }}>
          {cells}
        </Link>
      </TableRow>
    )
  }

  return <TableRow key={row.id}>{cells}</TableRow>
}
