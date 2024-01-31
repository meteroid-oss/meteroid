import { spaces } from '@md/foundation'
import { Flex, Skeleton, Table, cn } from '@md/ui'
import {
  ColumnDef,
  Row,
  flexRender,
  getCoreRowModel,
  useReactTable,
} from '@tanstack/react-table'
import { AlertCircleIcon } from 'lucide-react'
import { ReactNode, useMemo } from 'react'

interface SimpleTableProps<A> {
  columns: ColumnDef<A>[]
  data: A[] | undefined
  emptyMessage?: string | ReactNode
  isLoading?: boolean
  containerClassName?: string
  headTrClasses?: string
  bodyClassName?: string
}

export const SimpleTable = <A extends object>({
  columns,
  data,
  emptyMessage = 'No data',
  isLoading,
  containerClassName,
  headTrClasses,
  bodyClassName,
}: SimpleTableProps<A>) => {
  const defaultData = useMemo(() => [], [])

  const table = useReactTable({
    data: data ?? defaultData,
    columns,
    getCoreRowModel: getCoreRowModel(),
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
            className="h-14 whitespace-nowrap text-sm leading-5 text-gray-800"
          >
            <div className="flex items-center space-x-3 opacity-75">
              <AlertCircleIcon size={16} strokeWidth={2} />
              <div className="text-scale-1000 w-full">{emptyMessage}</div>
            </div>
          </Table.td>
        </Table.tr>
      )
    }

    return rows.map(standardRowRenderer)
  }, [isLoading, data, rows, standardRowRenderer, columns, emptyMessage])

  return (
    <Flex direction="column" gap={spaces.space9}>
      <Table
        head={table.getFlatHeaders().map(header => {
          const columnName = flexRender(header.column.columnDef.header, header.getContext())
          return <Table.th key={header.id}>{columnName}</Table.th>
        })}
        body={tableBody}
        containerClassName={cn('', containerClassName)}
        headTrClasses={headTrClasses}
        bodyClassName={bodyClassName}
        className={columns.length == 0 ? 'border-t border-slate-500' : ''}
      />
    </Flex>
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
