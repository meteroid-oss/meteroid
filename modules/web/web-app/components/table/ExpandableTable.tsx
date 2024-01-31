import { Table } from '@md/ui'
import {
  ColumnDef,
  OnChangeFn,
  PaginationState,
  Row,
  flexRender,
  getExpandedRowModel,
} from '@tanstack/react-table'
import { ChevronDownIcon, ChevronUpIcon } from 'lucide-react'
import { Fragment, useCallback } from 'react'

import { CustomTable } from '@/components/table/CustomTable'

interface ExpandableTableProps<A> {
  columns: ColumnDef<A>[]
  data: A[] | undefined
  sortable?: boolean
  pagination: PaginationState
  setPagination: OnChangeFn<PaginationState>
  totalCount: number
  emptyMessage?: string
  renderSubComponent: (props: { row: Row<A> }) => React.ReactElement
  isLoading?: boolean
}

/*
 *   To prevent a row from being expandable, have the property `isExpandable: false` in your data
 */
export const ExpandableTable = <A extends { isExpandable?: boolean }>({
  columns,
  data,
  sortable,
  pagination,
  setPagination,
  totalCount,
  emptyMessage = 'No data to display',
  renderSubComponent,
  isLoading,
}: ExpandableTableProps<A>) => {
  const expandableRowRenderer = useCallback(
    (row: Row<A>) => {
      return (
        <Fragment key={row.id}>
          <Table.tr>
            {row.getVisibleCells().map(cell => {
              return (
                <Table.td key={cell.id}>
                  {flexRender(cell.column.columnDef.cell, cell.getContext())}
                </Table.td>
              )
            })}
          </Table.tr>
          {row.getIsExpanded() && (
            <Table.tr className="sub-row">
              <Table.td colSpan={row.getVisibleCells().length}>
                {renderSubComponent({ row })}
              </Table.td>
            </Table.tr>
          )}
        </Fragment>
      )
    },
    [renderSubComponent]
  )
  return (
    <CustomTable
      columns={columns}
      data={data}
      sortable={sortable}
      pagination={pagination}
      setPagination={setPagination}
      totalCount={totalCount}
      emptyMessage={emptyMessage}
      rowRenderer={expandableRowRenderer}
      isLoading={isLoading}
      optionsOveride={{
        getExpandedRowModel: getExpandedRowModel(),

        getRowCanExpand: row =>
          row.original.isExpandable === undefined ? true : row.original.isExpandable,
      }}
    />
  )
}

export const expandColumn: ColumnDef<unknown> = {
  id: 'expander',
  header: () => null,
  cell: ({ row }) => {
    return row.getCanExpand() ? (
      <button
        {...{
          onClick: () => {
            console.log('BEFORE', row.getIsExpanded())
            row.toggleExpanded()
            console.log('AFTER', row.getIsExpanded())
          },
          style: { cursor: 'pointer' },
        }}
      >
        {row.getIsExpanded() ? <ChevronUpIcon size={14} /> : <ChevronDownIcon size={14} />}
      </button>
    ) : (
      ''
    )
  },
}
