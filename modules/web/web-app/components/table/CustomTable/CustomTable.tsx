import { ChevronDownIcon, ChevronUpIcon } from '@md/icons'
import { Skeleton, Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@md/ui'
import {
  ColumnDef,
  OnChangeFn,
  PaginationState,
  Row,
  SortingState,
  TableOptions,
  flexRender,
  getCoreRowModel,
  getExpandedRowModel,
  getSortedRowModel,
  useReactTable,
} from '@tanstack/react-table'
import { AlertCircleIcon } from 'lucide-react'
import { ReactNode, useMemo, useState } from 'react'

import { SortableDefaultIndicator, SortableIndicatorContainer } from './CustomTable.styled'
import Pagination from './components/Pagination'

interface CustomTableProps<A> {
  columns: ColumnDef<A>[]
  data: A[] | undefined
  sortable?: boolean
  sorting?: SortingState
  onSortingChange?: OnChangeFn<SortingState>
  emptyMessage?: string | ReactNode
  rowRenderer: (row: Row<A>) => JSX.Element
  optionsOveride?: Partial<TableOptions<A>>
  totalCount: number
  pagination: PaginationState
  setPagination: OnChangeFn<PaginationState>
  isLoading?: boolean
}

export const CustomTable = <A extends object>({
  columns,
  data,
  sortable = false,
  sorting: externalSorting,
  onSortingChange: externalOnSortingChange,
  emptyMessage,
  rowRenderer,
  optionsOveride,
  totalCount,
  pagination,
  setPagination,
  isLoading,
}: CustomTableProps<A>) => {
  const pageCount = useMemo(() => {
    if (totalCount === 0) return 0
    return Math.ceil(totalCount / pagination.pageSize)
  }, [pagination.pageSize, totalCount])

  const defaultData = useMemo(() => [], [])

  const [internalSorting, setInternalSorting] = useState<SortingState>([])

  const isManualSorting = !!externalOnSortingChange
  const sorting = isManualSorting ? (externalSorting ?? []) : internalSorting
  const setSorting = isManualSorting ? externalOnSortingChange : setInternalSorting

  // TanStack's getCanSort() requires accessorFn. For server-side sorting,
  // columns with enableSorting: true but no accessor need a stub.
  const resolvedColumns = useMemo(() => {
    if (!isManualSorting) return columns
    return columns.map(col =>
      col.enableSorting === true && !('accessorKey' in col) && !('accessorFn' in col)
        ? ({ ...col, accessorFn: () => null } as ColumnDef<A>)
        : col
    )
  }, [columns, isManualSorting])

  const table = useReactTable({
    data: data ?? defaultData,
    columns: resolvedColumns,
    pageCount: pageCount,
    state: {
      pagination,
      sorting: sortable ? sorting : undefined,
    },
    getCoreRowModel: getCoreRowModel(),
    manualPagination: true,
    manualSorting: isManualSorting,
    onPaginationChange: setPagination,
    getExpandedRowModel: getExpandedRowModel(),
    onSortingChange: sortable ? setSorting : undefined,
    getSortedRowModel: sortable && !isManualSorting ? getSortedRowModel() : undefined,

    ...optionsOveride,
  })

  const rows = table.getRowModel().rows

  const tableBody = useMemo(() => {
    if (isLoading) {
      const skeletonRows = pagination.pageSize
      const skeletonColumns = columns.length

      const skeletonRowsArray = Array.from({ length: skeletonRows })
      const skeletonColumnsArray = Array.from({ length: skeletonColumns })

      return skeletonRowsArray.map((_, rowIndex) => (
        <TableRow key={rowIndex}>
          {skeletonColumnsArray.map((_, colIndex) => {
            const header = columns[colIndex].header
            const isEmpty = typeof header === 'function'

            return (
              <TableCell key={colIndex}>
                {header && !isEmpty ? <Skeleton className="h-[16px] w-[100px]" /> : null}
              </TableCell>
            )
          })}
        </TableRow>
      ))
    }

    if (data && rows.length === 0) {
      return (
        <TableRow>
          <TableCell
            colSpan={columns.length}
            className="h-14 whitespace-nowrap border-t p-4 text-sm leading-5 text-muted-foreground"
          >
            <div className="flex items-center space-x-3 opacity-75">
              <AlertCircleIcon size={16} strokeWidth={2} />
              <p className="text-muted-foreground">{emptyMessage}</p>
            </div>
          </TableCell>
        </TableRow>
      )
    }

    return rows.map(rowRenderer)
  }, [isLoading, data, rows, rowRenderer, columns, emptyMessage, pagination.pageSize])

  return (
    <>
      <Table containerClassName="flex-1 grow">
        <TableHeader>
          <TableRow>
            {table.getFlatHeaders().map((header, headerIndex) => {
              const sortType = header.column.getIsSorted()
              const columnName = flexRender(header.column.columnDef.header, header.getContext())
              const isEmpty = typeof columnName === 'object'
              // eslint-disable-next-line @typescript-eslint/no-explicit-any
              const className: string | undefined = (header.column.columnDef as any).className
              const rowSortable = sortable && header.column.getCanSort()
              return (
                <TableHead key={header.id} className={className}>
                  {header.isPlaceholder ? null : rowSortable && !isEmpty ? (
                    <div
                      tabIndex={headerIndex}
                      data-sort={sortType}
                      className={rowSortable ? 'cursor-pointer select-none flex items-center' : ''}
                      onClick={rowSortable ? header.column.getToggleSortingHandler() : undefined}
                    >
                      <SortableIndicatorContainer>
                        {sortType === 'asc' ? (
                          <ChevronUpIcon size={14} data-type="chevron" />
                        ) : sortType === 'desc' ? (
                          <ChevronDownIcon size={14} data-type="chevron" />
                        ) : (
                          <SortableDefaultIndicator />
                        )}
                      </SortableIndicatorContainer>

                      {columnName}
                    </div>
                  ) : (
                    columnName
                  )}
                </TableHead>
              )
            })}
          </TableRow>
        </TableHeader>
        <TableBody>{tableBody}</TableBody>
      </Table>

      <Pagination
        pagination={pagination}
        setPagination={setPagination}
        totalCount={totalCount}
        isLoading={isLoading || false}
      />
    </>
  )
}
