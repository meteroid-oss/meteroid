import { ChevronLeftIcon, ChevronRightIcon } from '@md/icons'
import {
  Button,
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectLabel,
  SelectTrigger,
  Skeleton
} from '@md/ui'
import { OnChangeFn, PaginationState } from '@tanstack/react-table'
import { FunctionComponent } from 'react'

interface PaginationProps {
  pagination: PaginationState
  setPagination: OnChangeFn<PaginationState>
  totalCount: number
  isLoading: boolean
  /** `footer`: compact, right-aligned, for a TableCard footer (always shows the count). */
  variant?: 'reduced' | 'footer'
  showTotalOnSinglePage?: boolean
  /** Footer variant only: renders a page-size picker with these options. */
  pageSizeOptions?: number[]
}

const Pagination: FunctionComponent<PaginationProps> = ({
  pagination,
  setPagination,
  totalCount,
  isLoading,
  variant = 'reduced',
  showTotalOnSinglePage = false,
  pageSizeOptions,
}) => {
  const currentPage = pagination.pageIndex + 1
  const pageSize = pagination.pageSize
  const canPreviousPage = pagination.pageIndex > 0
  const totalPages = Math.ceil(totalCount / pageSize)
  const from = pagination.pageSize * pagination.pageIndex + 1
  const to = Math.min(pagination.pageSize * (pagination.pageIndex + 1), totalCount)
  const canNextPage = currentPage < totalPages
  const countLabel =
    totalPages > 1
      ? `${from.toLocaleString()}–${to.toLocaleString()} of ${totalCount.toLocaleString()}`
      : `${totalCount.toLocaleString()} result${totalCount !== 1 ? 's' : ''}`
  // The last page has the largest from/to, so its label is the widest this list can show.
  // Reserving its width keeps the arrows still while paging, without padding short lists.
  const widestCountLabel =
    totalPages > 1
      ? `${(pageSize * (totalPages - 1) + 1).toLocaleString()}–${totalCount.toLocaleString()} of ${totalCount.toLocaleString()}`
      : countLabel

  const countText = (
    // Inline display: SelectTrigger's `[&>span]:line-clamp-1` would otherwise override the grid.
    <span className="whitespace-nowrap text-center tabular-nums" style={{ display: 'grid' }}>
      <span className="col-start-1 row-start-1">{countLabel}</span>
      <span aria-hidden className="invisible col-start-1 row-start-1">
        {widestCountLabel}
      </span>
    </span>
  )

  // The count doubles as the page-size picker, so the control stays a single compact unit.
  const countPicker = pageSizeOptions && totalCount >= pageSizeOptions[0] && (
    <Select
      value={String(pageSize)}
      onValueChange={value =>
        setPagination({ ...pagination, pageSize: Number(value), pageIndex: 0 })
      }
    >
      <SelectTrigger
        className="h-full w-auto gap-1 rounded-none border-0 bg-transparent px-2 text-xs text-foreground shadow-none hover:bg-accent focus:ring-0 focus-visible:ring-1 focus-visible:ring-inset"
        aria-label={`${countLabel}, ${pageSize} rows per page`}
        title="Rows per page"
      >
        {countText}
      </SelectTrigger>
      <SelectContent align="end">
        <SelectGroup>
          <SelectLabel className="text-xs font-medium text-muted-foreground">
            Rows per page
          </SelectLabel>
          {pageSizeOptions.map(size => (
            <SelectItem key={size} value={String(size)} className="text-xs tabular-nums">
              {size}
            </SelectItem>
          ))}
        </SelectGroup>
      </SelectContent>
    </Select>
  )

  const handlePreviousPage = () => {
    if (!canPreviousPage) return
    setPagination({ ...pagination, pageIndex: pagination.pageIndex - 1 })
  }

  const handleNextPage = () => {
    if (!canNextPage) return
    setPagination({ ...pagination, pageIndex: pagination.pageIndex + 1 })
  }

  const control =
    !countPicker && totalPages <= 1 ? (
      <span className="text-xs tabular-nums text-muted-foreground">{countLabel}</span>
    ) : (
      <div className="inline-flex h-7 items-center overflow-hidden rounded-md border border-border text-xs">
        {totalPages > 1 && (
          <Button
            onClick={handlePreviousPage}
            disabled={!canPreviousPage}
            size="icon"
            variant="ghost"
            className="h-full w-7 rounded-none"
            aria-label="Previous page"
          >
            <ChevronLeftIcon size={14} />
          </Button>
        )}
        {countPicker || <span className="px-1 text-foreground">{countText}</span>}
        {totalPages > 1 && (
          <Button
            onClick={handleNextPage}
            disabled={!canNextPage}
            size="icon"
            variant="ghost"
            className="h-full w-7 rounded-none"
            aria-label="Next page"
          >
            <ChevronRightIcon size={14} />
          </Button>
        )}
      </div>
    )

  if (variant === 'footer') {
    if (isLoading && totalCount === 0) return <Skeleton className="h-4 w-16" />
    return control
  }

  // The standalone form of the footer control, for lists outside a TableCard.
  if (!countPicker && totalPages <= 1 && !showTotalOnSinglePage) return null
  return (
    <div className="flex justify-end py-2">
      {isLoading && totalCount === 0 ? <Skeleton className="h-7 w-24" /> : control}
    </div>
  )
}

export default Pagination;