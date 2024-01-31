import { spaces } from '@md/foundation'
import { ChevronUpIcon, ChevronDownIcon } from '@md/icons'
import { Flex, Skeleton, Table } from '@md/ui'
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

import {
  SortableDefaultIndicator,
  SortableIndicatorContainer,
  // SortableTh,
  // StyledTable,
  // StyledTd,
  // StyledTh,
} from './CustomTable.styled'
import Pagination from './components/Pagination'

interface CustomTableProps<A> {
  columns: ColumnDef<A>[]
  data: A[] | undefined
  sortable?: boolean
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

  const [sorting, setSorting] = useState<SortingState>([])
  const table = useReactTable({
    data: data ?? defaultData,
    columns,
    pageCount: pageCount,
    state: {
      pagination,
      sorting: sortable ? sorting : undefined,
    },
    getCoreRowModel: getCoreRowModel(),
    manualPagination: true,
    onPaginationChange: setPagination,
    getExpandedRowModel: getExpandedRowModel(),
    onSortingChange: sortable ? setSorting : undefined,
    getSortedRowModel: sortable ? getSortedRowModel() : undefined,

    ...optionsOveride,
  })

  const rows = table.getRowModel().rows

  const tableBody = useMemo(() => {
    if (isLoading) {
      const skeletonRows = 10
      const skeletonColumns = columns.length

      const skeletonRowsArray = Array.from({ length: skeletonRows })
      const skeletonColumnsArray = Array.from({ length: skeletonColumns })

      return skeletonRowsArray.map((_, rowIndex) => (
        <Table.tr key={rowIndex}>
          {skeletonColumnsArray.map((_, colIndex) => {
            const header = columns[colIndex].header
            const isEmpty = typeof header === 'function'

            return (
              <Table.td key={colIndex}>
                {header && !isEmpty ? <Skeleton height={16} width={100} /> : null}
              </Table.td>
            )
          })}
        </Table.tr>
      ))
    }

    if (data && rows.length === 0) {
      return (
        <Table.tr>
          <Table.td
            colSpan={columns.length}
            className="h-14 whitespace-nowrap border-t p-4 text-sm leading-5 text-gray-300"
          >
            <div className="flex items-center space-x-3 opacity-75">
              <AlertCircleIcon size={16} strokeWidth={2} />
              <p className="text-scale-1000">{emptyMessage}</p>
            </div>
          </Table.td>
        </Table.tr>
      )
    }

    return rows.map(rowRenderer)
  }, [isLoading, data, rows, rowRenderer, columns, emptyMessage])

  // useEffect(() => {
  //   const tableElement = document.querySelector('table') as HTMLTableElement
  //   const tableRect = tableElement.getBoundingClientRect()
  //   const tableBody = tableElement.querySelector('tbody') as HTMLTableSectionElement
  //   const paginationElement = document.getElementById('pagination') as HTMLDivElement
  //   const height = `calc(100vh - ${tableRect.top}px - ${paginationElement.offsetHeight}px - ${spaces.space12} * 2)`
  //
  //   tableBody.style.height = height
  // }, [])

  return (
    <Flex direction="column" gap={spaces.space9}>
      <Table
        head={table.getFlatHeaders().map((header, headerIndex) => {
          const sortType = header.column.getIsSorted()
          const columnName = flexRender(header.column.columnDef.header, header.getContext())
          const isEmpty = typeof columnName === 'object'
          const className: string | undefined = (header.column.columnDef as any).className
          const rowSortable = sortable && header.column.getCanSort()
          return (
            <Table.th key={header.id} className={className}>
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
            </Table.th>
          )
        })}
        body={tableBody}
      />
      <Pagination
        pagination={pagination}
        setPagination={setPagination}
        totalCount={totalCount}
        isLoading={isLoading || false}
      />
    </Flex>
  )
}
