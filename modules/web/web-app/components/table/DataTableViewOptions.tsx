import {
  Button,
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
  cn,
} from '@md/ui'
import { Column, Table } from '@tanstack/react-table'
import { RotateCcw, Settings2 } from 'lucide-react'

import { TableDensity } from '@/hooks/useTablePreferences'

// eslint-disable-next-line @typescript-eslint/no-explicit-any
const columnLabel = (column: Column<any, unknown>): string => {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const meta = column.columnDef.meta as any
  if (meta?.label) return meta.label
  const header = column.columnDef.header
  return typeof header === 'string' ? header : ''
}

interface DataTableViewOptionsProps<A> {
  table: Table<A>
  density: TableDensity
  onDensityChange: (density: TableDensity) => void
  onReset: () => void
  /** When true, expose a toggle for the list's metric chart. */
  hasChart?: boolean
  showChart?: boolean
  onShowChartChange?: (show: boolean) => void
}

const DENSITIES: TableDensity[] = ['compact', 'comfortable']

export const DataTableViewOptions = <A,>({
  table,
  density,
  onDensityChange,
  onReset,
  hasChart = false,
  showChart = true,
  onShowChartChange,
}: DataTableViewOptionsProps<A>) => {
  const hideableColumns = table
    .getAllColumns()
    .filter(column => column.getCanHide() && columnLabel(column) !== '')

  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button
          variant="ghost"
          size="icon"
          className="h-8 w-8 text-muted-foreground hover:text-foreground"
          aria-label="View options"
        >
          <Settings2 size={15} />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-56">
        <DropdownMenuLabel className="text-xs font-normal text-muted-foreground">
          Row height
        </DropdownMenuLabel>
        <div className="flex gap-1 px-1.5 pb-1.5">
          {DENSITIES.map(value => (
            <button
              key={value}
              onClick={() => onDensityChange(value)}
              className={cn(
                'flex-1 rounded-md px-2 py-1 text-xs capitalize transition-colors',
                density === value
                  ? 'bg-secondary text-foreground'
                  : 'text-muted-foreground hover:bg-muted hover:text-foreground'
              )}
            >
              {value}
            </button>
          ))}
        </div>
        {hasChart && (
          <>
            <DropdownMenuSeparator />
            <DropdownMenuCheckboxItem
              checked={showChart}
              onCheckedChange={value => onShowChartChange?.(!!value)}
              onSelect={event => event.preventDefault()}
            >
              Show chart
            </DropdownMenuCheckboxItem>
          </>
        )}
        <DropdownMenuSeparator />
        <DropdownMenuLabel className="text-xs font-normal text-muted-foreground">
          Columns
        </DropdownMenuLabel>
        <div className="max-h-72 overflow-y-auto">
          {hideableColumns.map(column => (
            <DropdownMenuCheckboxItem
              key={column.id}
              checked={column.getIsVisible()}
              onCheckedChange={value => column.toggleVisibility(!!value)}
              onSelect={event => event.preventDefault()}
            >
              {columnLabel(column)}
            </DropdownMenuCheckboxItem>
          ))}
        </div>
        <DropdownMenuSeparator />
        <DropdownMenuItem onClick={onReset} className="text-xs text-muted-foreground">
          <RotateCcw size={13} className="mr-2" />
          Reset to defaults
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  )
}
