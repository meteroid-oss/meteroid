import { ChevronDownIcon, ChevronUpIcon } from '@md/icons'
import {
  Checkbox,
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
  cn,
} from '@md/ui'
import {
  ColumnDef,
  OnChangeFn,
  PaginationState,
  Row,
  RowSelectionState,
  SortingState,
  TableOptions,
  VisibilityState,
  flexRender,
  getCoreRowModel,
  getExpandedRowModel,
  getSortedRowModel,
  useReactTable,
} from '@tanstack/react-table'
import { AlertCircleIcon } from 'lucide-react'
import { ReactNode, useEffect, useMemo, useRef, useState } from 'react'
import { createPortal } from 'react-dom'

import { DataTableViewOptions } from '@/components/table/DataTableViewOptions'
import {
  TableToolbarPortal,
  useTableFooterSlot,
  useTableToolbarSlot,
} from '@/components/table/TableToolbarSlot'
import { PAGE_SIZE_OPTIONS, TableDensity } from '@/hooks/useTablePreferences'

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
  columnVisibility?: VisibilityState
  onColumnVisibilityChange?: OnChangeFn<VisibilityState>
  density?: TableDensity
  onDensityChange?: (density: TableDensity) => void
  showViewOptions?: boolean
  onResetPreferences?: () => void
  /** Optional full-width strip (e.g. a metric sparkline) shown above the rows. */
  chart?: ReactNode
  /** Whether the chart is currently shown; toggled from the view-options menu. */
  showChart?: boolean
  onShowChartChange?: (show: boolean) => void
  enableSelection?: boolean
  getRowId?: (originalRow: A, index: number) => string
  renderSelectionActions?: (rows: Row<A>[]) => ReactNode
}

const MAX_SKELETON_ROWS = 10

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
  columnVisibility,
  onColumnVisibilityChange,
  density = 'compact',
  onDensityChange,
  showViewOptions,
  onResetPreferences,
  chart,
  showChart = true,
  onShowChartChange,
  enableSelection = false,
  getRowId,
  renderSelectionActions,
}: CustomTableProps<A>) => {
  const pageCount = useMemo(() => {
    if (totalCount === 0) return 0
    return Math.ceil(totalCount / pagination.pageSize)
  }, [pagination.pageSize, totalCount])

  // A stale `?page=` or an emptied last page lands past the end. With a known total, snap to
  // the last page. Server-paged lists report total 0 past the end, so the last page is unknown:
  // step back once (usually the last page's rows were just removed), then fall back to page 1.
  const steppedBack = useRef(false)
  useEffect(() => {
    if (isLoading || pagination.pageIndex === 0) return
    if (pageCount > 0) {
      steppedBack.current = false
      if (pagination.pageIndex >= pageCount) {
        setPagination({ ...pagination, pageIndex: pageCount - 1 })
      }
    } else if (data?.length === 0) {
      setPagination({ ...pagination, pageIndex: steppedBack.current ? 0 : pagination.pageIndex - 1 })
      steppedBack.current = true
    }
  }, [isLoading, pageCount, data?.length, pagination, setPagination])

  const defaultData = useMemo(() => [], [])

  const [internalSorting, setInternalSorting] = useState<SortingState>([])
  const [rowSelection, setRowSelection] = useState<RowSelectionState>({})

  const isManualSorting = !!externalOnSortingChange
  const sorting = isManualSorting ? (externalSorting ?? []) : internalSorting
  const setSorting = isManualSorting ? externalOnSortingChange : setInternalSorting

  // TanStack's getCanSort() requires accessorFn. For server-side sorting,
  // columns with enableSorting: true but no accessor need a stub.
  const resolvedColumns = useMemo(() => {
    const stubbed = !isManualSorting
      ? columns
      : columns.map(col =>
          col.enableSorting === true && !('accessorKey' in col) && !('accessorFn' in col)
            ? ({ ...col, accessorFn: () => null } as ColumnDef<A>)
            : col
        )
    if (!enableSelection) return stubbed
    const selectionColumn: ColumnDef<A> = {
      id: '__select__',
      enableSorting: false,
      enableHiding: false,
      meta: { skipLink: true },
      header: ({ table }) => (
        <Checkbox
          checked={
            table.getIsAllPageRowsSelected()
              ? true
              : table.getIsSomePageRowsSelected()
                ? 'indeterminate'
                : false
          }
          onCheckedChange={value => table.toggleAllPageRowsSelected(!!value)}
          aria-label="Select all rows"
        />
      ),
      cell: ({ row }) => (
        <Checkbox
          checked={row.getIsSelected()}
          onCheckedChange={value => row.toggleSelected(!!value)}
          onClick={e => e.stopPropagation()}
          aria-label="Select row"
        />
      ),
    }
    return [selectionColumn, ...stubbed]
  }, [columns, isManualSorting, enableSelection])

  const table = useReactTable({
    data: data ?? defaultData,
    columns: resolvedColumns,
    pageCount: pageCount,
    state: {
      pagination,
      sorting: sortable ? sorting : undefined,
      columnVisibility,
      rowSelection,
    },
    getCoreRowModel: getCoreRowModel(),
    manualPagination: true,
    manualSorting: isManualSorting,
    enableRowSelection: enableSelection,
    onPaginationChange: setPagination,
    onColumnVisibilityChange,
    onRowSelectionChange: setRowSelection,
    getRowId,
    getExpandedRowModel: getExpandedRowModel(),
    onSortingChange: sortable ? setSorting : undefined,
    getSortedRowModel: sortable && !isManualSorting ? getSortedRowModel() : undefined,

    ...optionsOveride,
  })

  const rows = table.getRowModel().rows
  const selectedRows = table.getSelectedRowModel().rows
  const toolbarSlot = useTableToolbarSlot()
  const footerSlot = useTableFooterSlot()

  const tableBody = useMemo(() => {
    if (isLoading) {
      // One animation per row, not per cell: each animated element costs style and compositing work.
      const skeletonRowsArray = Array.from({ length: Math.min(pagination.pageSize, MAX_SKELETON_ROWS) })

      return skeletonRowsArray.map((_, rowIndex) => (
        <TableRow key={rowIndex} className="animate-pulse">
          {columns.map((column, colIndex) => {
            const header = column.header
            const isEmpty = typeof header === 'function'

            return (
              <TableCell key={colIndex}>
                {header && !isEmpty ? (
                  <div className="h-[16px] w-[100px] rounded-md bg-accent/50" />
                ) : null}
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
      {enableSelection && selectedRows.length > 0 && (
        <div className="flex items-center gap-3 rounded-md border bg-muted/40 px-3 py-2 mb-1 text-sm">
          <span className="font-medium">{selectedRows.length} selected</span>
          {renderSelectionActions?.(selectedRows)}
          <button
            type="button"
            className="ml-auto text-xs text-muted-foreground hover:text-foreground"
            onClick={() => table.resetRowSelection()}
          >
            Clear
          </button>
        </div>
      )}
      {showViewOptions &&
        (toolbarSlot ? (
          <TableToolbarPortal>
            <DataTableViewOptions
              table={table}
              density={density}
              onDensityChange={onDensityChange ?? (() => undefined)}
              onReset={onResetPreferences ?? (() => undefined)}
              hasChart={!!chart}
              showChart={showChart}
              onShowChartChange={onShowChartChange}
            />
          </TableToolbarPortal>
        ) : (
          <div className="flex items-center justify-end pb-1">
            <DataTableViewOptions
              table={table}
              density={density}
              onDensityChange={onDensityChange ?? (() => undefined)}
              onReset={onResetPreferences ?? (() => undefined)}
              hasChart={!!chart}
              showChart={showChart}
              onShowChartChange={onShowChartChange}
            />
          </div>
        ))}
      {/* Full-bleed (cancels the body's px-1) so the strip and its own bottom border
          span the card edge-to-edge. The strip draws the border only when it has data,
          so an empty/locked metric leaves no stray divider. */}
      {chart && showChart && <div className="-mx-1">{chart}</div>}
      <Table
        containerClassName="flex-1 grow"
        className={cn(
          // Uniform row height so cells with a stacked secondary line (e.g. a
          // date plus relative time) don't make some rows taller than others;
          // single-line content vertically centers to the same baseline.
          '[&_tbody_tr]:h-[52px]',
          density === 'compact' &&
            '[&_td]:!py-1 [&_th]:!h-8 [&_tbody_tr]:!h-10 [&_td]:!text-xs [&_th]:!text-xs'
        )}
      >
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

      {footerSlot ? (
        createPortal(
          <Pagination
            pagination={pagination}
            setPagination={setPagination}
            totalCount={totalCount}
            isLoading={isLoading || false}
            variant="footer"
            pageSizeOptions={PAGE_SIZE_OPTIONS}
          />,
          footerSlot
        )
      ) : (
        <Pagination
          pagination={pagination}
          setPagination={setPagination}
          totalCount={totalCount}
          isLoading={isLoading || false}
        />
      )}
    </>
  )
}
