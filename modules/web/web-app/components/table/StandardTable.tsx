import { TableCell, TableRow } from '@md/ui'
import {
  ColumnDef,
  OnChangeFn,
  VisibilityState,
  PaginationState,
  Row,
  SortingState,
  flexRender,
} from '@tanstack/react-table'
import { ReactNode, useRef } from 'react'
import { Link } from 'react-router-dom'

import { CustomTable } from '@/components/table/CustomTable'
import { useTableDensity, useTablePreferences } from '@/hooks/useTablePreferences'

interface StandardTableProps<A> {
  columns: ColumnDef<A>[]
  data: A[] | undefined
  sortable?: boolean
  sorting?: SortingState
  onSortingChange?: OnChangeFn<SortingState>
  pagination: PaginationState
  setPagination: OnChangeFn<PaginationState>
  totalCount: number
  emptyMessage?: string | ReactNode
  isLoading?: boolean
  rowLink?: (row: Row<A>) => string
  /** Called when a row is hovered for a moment or focused, to prefetch what its link opens. */
  onRowIntent?: (row: Row<A>) => void
  rowClassName?: (row: Row<A>) => string
  // When set, enables per-user column visibility (persisted under this key) and
  // the discreet view-options menu. Omit to keep the plain table. Row height is
  // a separate global preference and applies regardless of this key.
  tableKey?: string
  defaultHidden?: string[]
  enableSelection?: boolean
  getRowId?: (originalRow: A, index: number) => string
  renderSelectionActions?: (rows: Row<A>[]) => ReactNode
  /** Optional full-width metric strip above the rows, toggled from the view-options
   *  menu and remembered per table (needs `tableKey`). */
  chart?: ReactNode
  /** Externally controlled column visibility (e.g. an active saved view). Overrides the
   *  per-user preference while set; needs `tableKey`. */
  columnState?: ColumnState
}

export interface ColumnState {
  visibility: VisibilityState
  onChange: OnChangeFn<VisibilityState>
  onReset: () => void
}

export const StandardTable = <A extends object>({
  columns,
  data,
  sortable,
  sorting,
  onSortingChange,
  pagination,
  setPagination,
  totalCount,
  emptyMessage = 'No data to display',
  isLoading,
  rowLink,
  onRowIntent,
  rowClassName,
  tableKey,
  defaultHidden,
  enableSelection,
  getRowId,
  renderSelectionActions,
  chart,
  columnState,
}: StandardTableProps<A>) => {
  const enabled = !!tableKey
  const { columnVisibility, setColumnVisibility, showChart, setShowChart, reset } =
    useTablePreferences(tableKey ?? '', {
      defaultHidden,
    })
  const { density, setDensity } = useTableDensity()

  return (
    <CustomTable
      columns={columns}
      data={data}
      sortable={sortable}
      sorting={sorting}
      onSortingChange={onSortingChange}
      pagination={pagination}
      setPagination={setPagination}
      totalCount={totalCount}
      emptyMessage={emptyMessage}
      rowRenderer={row => standardRowRenderer(row, rowLink, rowClassName, onRowIntent)}
      isLoading={isLoading}
      columnVisibility={enabled ? (columnState?.visibility ?? columnVisibility) : undefined}
      onColumnVisibilityChange={
        enabled ? (columnState?.onChange ?? setColumnVisibility) : undefined
      }
      density={density}
      onDensityChange={setDensity}
      showViewOptions={enabled}
      onResetPreferences={
        columnState
          ? () => {
              columnState.onReset()
              setShowChart(true)
            }
          : reset
      }
      enableSelection={enableSelection}
      getRowId={getRowId}
      renderSelectionActions={renderSelectionActions}
      chart={chart}
      showChart={showChart}
      onShowChartChange={setShowChart}
    />
  )
}

const standardRowRenderer = <A extends object>(
  row: Row<A>,
  rowLink?: (row: Row<A>) => string,
  rowClassName?: (row: Row<A>) => string,
  onRowIntent?: (row: Row<A>) => void
) => {
  const cells = row.getVisibleCells().map(cell => {
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const colDef = cell.column.columnDef as any
    const skipLink = colDef.meta?.skipLink === true
    return (
      <TableCell key={cell.id} className={colDef.className}>
        {rowLink && !skipLink ? (
          <Link to={rowLink(row)} className="block w-full h-full align-middle min-h-[20px] min-w-1">
            {flexRender(cell.column.columnDef.cell, cell.getContext())}
          </Link>
        ) : (
          flexRender(cell.column.columnDef.cell, cell.getContext())
        )}
      </TableCell>
    )
  })

  if (onRowIntent) {
    return (
      <IntentRow key={row.id} className={rowClassName?.(row)} onIntent={() => onRowIntent(row)}>
        {cells}
      </IntentRow>
    )
  }

  return (
    <TableRow key={row.id} className={rowClassName?.(row)}>
      {cells}
    </TableRow>
  )
}

// Long enough to skip rows the pointer only sweeps across.
const HOVER_INTENT_MS = 80

const IntentRow = ({
  className,
  onIntent,
  children,
}: {
  className?: string
  onIntent: () => void
  children: ReactNode
}) => {
  const timer = useRef<ReturnType<typeof setTimeout>>(undefined)
  const cancel = () => clearTimeout(timer.current)

  return (
    <TableRow
      className={className}
      onPointerEnter={() => {
        cancel()
        timer.current = setTimeout(onIntent, HOVER_INTENT_MS)
      }}
      onPointerLeave={cancel}
      onPointerDown={onIntent}
      onFocus={onIntent}
    >
      {children}
    </TableRow>
  )
}
