import { Checkbox, Table, TableBody, TableCell, TableHead, TableHeader, TableRow, cn } from '@md/ui'
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
import { ReactNode, useEffect, useMemo, useRef, useState, type JSX } from 'react'
import { createPortal } from 'react-dom'

import { DataTableViewOptions } from '@/components/table/DataTableViewOptions'
import { SelectionToolbar } from '@/components/table/SelectionToolbar'
import {
  TableToolbarPortal,
  useTableFooterSlot,
  useTableToolbarSlot,
} from '@/components/table/TableToolbarSlot'
import { SELECT_COLUMN_ID, columnClassName } from '@/components/table/actionColumn'
import { PAGE_SIZE_OPTIONS, TableDensity } from '@/hooks/useTablePreferences'

import {
  SortableDefaultIndicator,
  SortableIndicatorContainer,
  SortedIndicator,
} from './CustomTable.styled'
import Pagination from './components/Pagination'

interface CustomTableProps<A> {
  columns: ColumnDef<A>[]
  data: A[] | undefined
  sortable?: boolean
  sorting?: SortingState
  onSortingChange?: OnChangeFn<SortingState>
  emptyMessage?: string | ReactNode
  rowRenderer: (row: Row<A>, index: number, rows: Row<A>[]) => JSX.Element
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
  renderSelectionActions?: (rows: Row<A>[], clearSelection: () => void) => ReactNode
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
      setPagination({
        ...pagination,
        pageIndex: steppedBack.current ? 0 : pagination.pageIndex - 1,
      })
      steppedBack.current = true
    }
  }, [isLoading, pageCount, data?.length, pagination, setPagination])

  const defaultData = useMemo(() => [], [])

  const [internalSorting, setInternalSorting] = useState<SortingState>([])
  const [rowSelection, setRowSelection] = useState<RowSelectionState>({})
  // Shift-select: the row last ticked or shift-clicked, which a shift-click extends a range from.
  const selectionAnchor = useRef<string | null>(null)
  const pickRef = useRef<(rowId: string, shiftKey: boolean) => boolean>(() => false)

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
      id: SELECT_COLUMN_ID,
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
          onClick={e => {
            e.stopPropagation()
            // With shift, every row from the last one ticked to this one takes this one's new state.
            if (pickRef.current(row.id, e.shiftKey)) e.preventDefault()
          }}
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

  /** Ticks or unticks a row by its id; with shift and an earlier row ticked, the whole range
   *  between them, in the order shown, takes the clicked row's new state. Returns whether it
   *  handled the range (the checkbox then skips its own toggle). */
  pickRef.current = (rowId, shiftKey) => {
    const anchor = selectionAnchor.current
    selectionAnchor.current = rowId
    const from = anchor === null ? -1 : rows.findIndex(r => r.id === anchor)
    const to = rows.findIndex(r => r.id === rowId)
    if (!shiftKey || from < 0 || to < 0 || from === to) return false
    const value = !rows[to].getIsSelected()
    const [start, end] = from < to ? [from, to] : [to, from]
    setRowSelection(prev => {
      const next = { ...prev }
      for (const r of rows.slice(start, end + 1)) {
        if (!r.getCanSelect()) continue
        if (value) next[r.id] = true
        else delete next[r.id]
      }
      return next
    })
    return true
  }

  // Shift-click anywhere on a row selects it (or the range up to it) instead of opening it.
  // Buttons and fields in the row keep their own click.
  const rowFromShiftClick = (e: React.MouseEvent) => {
    if (!enableSelection || !e.shiftKey) return null
    const target = e.target as HTMLElement
    if (target.closest('button, input, select, textarea, [role="checkbox"], [role="menuitem"]'))
      return null
    return target.closest<HTMLElement>('tr[data-row-id]')?.dataset.rowId ?? null
  }
  const onBodyClickCapture = (e: React.MouseEvent) => {
    const rowId = rowFromShiftClick(e)
    if (!rowId) return
    // Before the row's link sees it, so nothing opens.
    e.preventDefault()
    e.stopPropagation()
    if (!pickRef.current(rowId, true)) {
      const row = rows.find(r => r.id === rowId)
      if (row?.getCanSelect()) row.toggleSelected()
    }
  }
  // Shift-pressing would otherwise extend the page's text selection across the rows.
  const onBodyMouseDown = (e: React.MouseEvent) => {
    if (rowFromShiftClick(e)) e.preventDefault()
  }
  const selectedRows = table.getSelectedRowModel().rows
  const toolbarSlot = useTableToolbarSlot()
  const footerSlot = useTableFooterSlot()

  // Not memoized: rows read the table's state (selection, expansion) when they render.
  const tableBody = (() => {
    if (isLoading) {
      // One animation per row, not per cell: each animated element costs style and compositing work.
      const skeletonRowsArray = Array.from({
        length: Math.min(pagination.pageSize, MAX_SKELETON_ROWS),
      })

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
            className="h-14 whitespace-nowrap border-t-[0.5px] border-foreground/[0.102] p-4 text-sm leading-5 text-muted-foreground"
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
  })()


  return (
    <>
      {enableSelection && (
        <SelectionToolbar
          count={selectedRows.length}
          footerSlot={footerSlot}
          onClear={() => table.resetRowSelection()}
        >
          {selectedRows.length > 0 &&
            renderSelectionActions?.(selectedRows, () => table.resetRowSelection())}
        </SelectionToolbar>
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
      {/* Spans the card edge-to-edge. The strip draws its bottom border only when it has data,
          so an empty/locked metric leaves no stray divider. */}
      {chart && showChart && <div className="shrink-0">{chart}</div>}
      <Table
        containerClassName="min-h-0 flex-1 grow"
        className={cn(
          // Uniform row height so cells with a stacked secondary line (e.g. a
          // date plus relative time) don't make some rows taller than others;
          // single-line content vertically centers to the same baseline.
          '[&_tbody_tr]:h-[52px]',
          // Rows run edge to edge with a 24px inset on the left; columns sit 20px apart.
          '[&_td]:px-2.5 [&_th]:px-2.5 [&_td:first-child]:pl-6 [&_th:first-child]:pl-6 [&_td:last-child]:pr-4 [&_th:last-child]:pr-4',
          // Column headers 32px in every density.
          '[&_th]:!h-8 [&_th]:font-medium [&_th]:text-[13px] [&_th]:text-foreground/60 [&_td]:text-[13px]',
          // Row dividers at white 10.2%, and a light tint on hovered and selected rows (a step more when
          // both), all visible on the table card.
          // Hairlines: 0.5px under the header and between rows (the last row keeps none). Drawn
          // on the cells of a separated table: a collapsed one rounds row borders to whole pixels.
          'border-separate border-spacing-0 [&_tr]:!border-b-0 [&_thead_th]:border-b-[0.5px] [&_tbody_tr:not(:last-child)>td]:border-b-[0.5px] [&_th]:border-foreground/[0.102] [&_td]:border-foreground/[0.102] [&_tbody_tr:hover]:bg-foreground/[0.04] [&_tbody_tr[data-state=selected]]:bg-foreground/[0.04] [&_tbody_tr[data-state=selected]:hover]:bg-foreground/[0.06]',
          density === 'compact' && '[&_td]:!py-1 [&_tbody_tr]:!h-11'
        )}
      >
        <TableHeader className="border-t-0 bg-transparent">
          <TableRow>
            {table.getFlatHeaders().map((header, headerIndex) => {
              const sortType = header.column.getIsSorted()
              const columnName = flexRender(header.column.columnDef.header, header.getContext())
              const isEmpty = typeof columnName === 'object'
              const className = columnClassName(header.column.columnDef)
              const rowSortable = sortable && header.column.getCanSort()
              return (
                <TableHead key={header.id} className={className}>
                  {header.isPlaceholder ? null : rowSortable && !isEmpty ? (
                    <div
                      tabIndex={headerIndex}
                      data-sort={sortType}
                      className={
                        rowSortable ? 'cursor-pointer select-none flex items-center gap-1.5' : ''
                      }
                      onClick={rowSortable ? header.column.getToggleSortingHandler() : undefined}
                    >
                      <SortableIndicatorContainer>
                        {sortType ? (
                          <SortedIndicator direction={sortType} />
                        ) : (
                          <SortableDefaultIndicator />
                        )}
                      </SortableIndicatorContainer>

                      {columnName}
                    </div>
                  ) : (
                    <>
                      {columnName}
                    </>
                  )}
                </TableHead>
              )
            })}
          </TableRow>
        </TableHeader>
        <TableBody onClickCapture={onBodyClickCapture} onMouseDown={onBodyMouseDown}>
          {tableBody}
        </TableBody>
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
