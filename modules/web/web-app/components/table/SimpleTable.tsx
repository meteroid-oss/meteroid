import { Skeleton, Table, TableRow, TableCell, TableHeader, TableBody, TableHead, cn } from '@md/ui'
import { ColumnDef, Row, flexRender, getCoreRowModel, useReactTable } from '@tanstack/react-table'
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
        <TableRow key={rowIndex}>
          {skeletonColumnsArray.map((_, colIndex) => {
            const header = columns[colIndex].header
            const isEmpty = typeof header === 'function'

            return (
              <TableCell key={colIndex}>
                {header && !isEmpty ? <Skeleton className="h-4 w-[100px]" /> : null}
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
            className="h-14 whitespace-nowrap text-sm leading-5 text-muted-foreground "
          >
            <div className=" h-full w-full flex flex-col gap-4 items-center justify-center ">
              <div className="flex flex-row gap-2">
                <AlertCircleIcon size={16} strokeWidth={2} />
                <div className=" text-sm font-medium ">{emptyMessage}</div>
              </div>
            </div>
          </TableCell>
        </TableRow>
      )
    }

    return rows.map(standardRowRenderer)
  }, [isLoading, data, rows, columns, emptyMessage])

  return (
    <div className="flex flex-col gap-8">
      <Table
        className={cn(containerClassName, columns.length == 0 ? 'border-t border-border' : '')}
      >
        <TableHeader className={headTrClasses}>
          <TableRow>
            {table.getFlatHeaders().map(header => {
              const columnName = flexRender(header.column.columnDef.header, header.getContext())
              return <TableHead key={header.id}>{columnName}</TableHead>
            })}
          </TableRow>
        </TableHeader>
        <TableBody className={bodyClassName}>{tableBody}</TableBody>
      </Table>
    </div>
  )
}

const standardRowRenderer = <A extends object>(row: Row<A>) => {
  return (
    <TableRow key={row.id}>
      {row.getVisibleCells().map(cell => {
        return (
          <TableCell key={cell.id}>
            {flexRender(cell.column.columnDef.cell, cell.getContext())}
          </TableCell>
        )
      })}
    </TableRow>
  )
}
