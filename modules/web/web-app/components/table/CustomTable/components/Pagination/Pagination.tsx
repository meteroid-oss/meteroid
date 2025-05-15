import { ChevronLeftIcon, ChevronRightIcon } from '@md/icons'
import {
  Button,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Skeleton,
  cn
} from '@md/ui'
import { OnChangeFn, PaginationState } from '@tanstack/react-table'
import { Fragment, FunctionComponent } from 'react'

interface PaginationProps {
  pagination: PaginationState
  setPagination: OnChangeFn<PaginationState>
  totalCount: number
  isLoading: boolean
  variant?: 'default' | 'reduced'
  showTotalOnSinglePage?: boolean
}

const Pagination: FunctionComponent<PaginationProps> = ({
  pagination,
  setPagination,
  totalCount,
  isLoading,
  variant = 'reduced',
  showTotalOnSinglePage = false,
}) => {
  const currentPage = pagination.pageIndex + 1
  const pageSize = pagination.pageSize
  const canPreviousPage = pagination.pageIndex > 0
  const totalPages = Math.ceil(totalCount / pageSize)
  const from = pagination.pageSize * pagination.pageIndex + 1
  const to = Math.min(pagination.pageSize * (pagination.pageIndex + 1), totalCount)
  const canNextPage = currentPage < totalPages

  // Only show page numbers for a reasonable range around current page
  const getPageRange = (current: number, total: number) => {
    if (total <= 7) {
      return Array.from({ length: total }, (_, i) => i + 1)
    }
    
    if (current <= 3) {
      return [1, 2, 3, 4, '...', total]
    }
    
    if (current >= total - 2) {
      return [1, '...', total - 3, total - 2, total - 1, total]
    }
    
    return [1, '...', current - 1, current, current + 1, '...', total]
  }

  const handlePreviousPage = () => {
    if (!canPreviousPage) return
    setPagination({ ...pagination, pageIndex: pagination.pageIndex - 1 })
  }

  const handleNextPage = () => {
    if (!canNextPage) return
    setPagination({ ...pagination, pageIndex: pagination.pageIndex + 1 })
  }

  // If we have only one page and shouldn't show just the total, return nothing
  if (totalPages <= 1 && !showTotalOnSinglePage) return null;
  
  // If we have only one page but should show the total, render just the total count
  if (totalPages <= 1 && showTotalOnSinglePage) {
    return (
      <div className="flex justify-end text-sm text-muted-foreground py-2">
        {isLoading ? (
          <Skeleton className="h-6 w-32" />
        ) : (
          <span>{totalCount} result{totalCount !== 1 ? 's' : ''}</span>
        )}
      </div>
    );
  }

  // For the reduced variant - simplified version
  if (variant === 'reduced') {
    return (
      <div className="flex items-center justify-between py-2 text-sm">
        {isLoading ? (
          <Skeleton className="h-6 w-32" />
        ) : (
          <>
            <div className="flex items-center space-x-1">
              <Button
                onClick={handlePreviousPage}
                disabled={!canPreviousPage}
                size="icon"
                variant="ghost"
                className="h-7 w-7"
              >
                <ChevronLeftIcon size={14} />
              </Button>
              <span className="text-xs px-2">
                {currentPage} / {totalPages}
              </span>
              <Button
                onClick={handleNextPage}
                disabled={!canNextPage}
                size="icon"
                variant="ghost"
                className="h-7 w-7"
              >
                <ChevronRightIcon size={14} />
              </Button>
            </div>
            <span className="text-xs text-muted-foreground">
              {totalCount} result{totalCount !== 1 ? 's' : ''}
            </span>
          </>
        )}
      </div>
    );
  }

  // Default full pagination
  return (
    <nav className="py-2">
      <div className="flex items-center justify-between">
        {!isLoading ? (
          <>
            <div className="flex items-center space-x-1">
              <Button
                onClick={handlePreviousPage}
                disabled={!canPreviousPage}
                size="icon"
                variant="ghost"
                className="h-8 w-8"
              >
                <ChevronLeftIcon size={16} />
              </Button>

              <div className="flex items-center">
                {getPageRange(currentPage, totalPages).map((page, index) => (
                  <Fragment key={index}>
                    {page === '...' ? (
                      <span className="px-2 text-muted-foreground">...</span>
                    ) : (
                      <Button
                        size="sm"
                        variant={page === currentPage ? 'default' : 'ghost'}
                        className={cn(
                          "h-8 w-8 p-0 font-normal",
                          page === currentPage && "font-medium"
                        )}
                        onClick={() => 
                          setPagination({ ...pagination, pageIndex: Number(page) - 1 })
                        }
                      >
                        {page}
                      </Button>
                    )}
                  </Fragment>
                ))}
              </div>

              <Button
                onClick={handleNextPage}
                disabled={!canNextPage}
                size="icon"
                variant="ghost"
                className="h-8 w-8"
              >
                <ChevronRightIcon size={16} />
              </Button>
            </div>

            <div className="flex items-center space-x-2 text-sm">
              <span className="text-muted-foreground">
                {from}-{to} of {totalCount}
              </span>

              <Select
                value={String(pageSize)}
                onValueChange={value =>
                  setPagination({ ...pagination, pageSize: Number(value), pageIndex: 0 })
                }
              >
                <SelectTrigger className="h-8 w-20">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {[10, 25, 50, 100].map(size => (
                    <SelectItem key={size} value={String(size)}>
                      {size}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
          </>
        ) : (
          <>
            <Skeleton className="h-8 w-32" />
            <Skeleton className="h-8 w-24" />
          </>
        )}
      </div>
    </nav>
  );
};

export default Pagination;