import { spaces } from '@md/foundation'
import { ChevronLeftIcon, ChevronRightIcon } from '@md/icons'
import { Button, Flex, Select, SelectItem, Skeleton } from '@md/ui'
import { Fragment, FunctionComponent } from 'react'

import { CountInfo } from './Pagination.styled'
import { getPageRange } from './Pagination.utils'

import type { OnChangeFn, PaginationState } from '@tanstack/react-table'

interface PaginationProps {
  pagination: PaginationState
  setPagination: OnChangeFn<PaginationState>
  totalCount: number
  isLoading: boolean
}

const Pagination: FunctionComponent<PaginationProps> = ({
  pagination,
  setPagination,
  totalCount,
  isLoading,
}) => {
  const currentPage = pagination.pageIndex + 1
  const pageSize = pagination.pageSize
  const canPreviousPage = pagination.pageIndex > 0
  const totalPages = Math.ceil(totalCount / pageSize)
  const from = pagination.pageSize * pagination.pageIndex + 1
  const to = Math.min(pagination.pageSize * (pagination.pageIndex + 1), totalCount)
  const canNextPage = currentPage < totalPages

  const handlePreviousPage = () => {
    if (!canPreviousPage) return

    setPagination({ ...pagination, pageIndex: pagination.pageIndex - 1 })
  }

  const handleNextPage = () => {
    if (!canNextPage) return

    setPagination({ ...pagination, pageIndex: pagination.pageIndex + 1 })
  }

  return (
    <nav id="pagination">
      <Flex direction="row" align="center" justify="space-between">
        {!isLoading ? (
          <Fragment>
            <Flex direction="row" gap={spaces.space4} align="flex-end">
              <Button
                onClick={handlePreviousPage}
                disabled={!canPreviousPage}
                title="Previous Page"
                size="small"
                variant="tertiary"
              >
                <ChevronLeftIcon size={16} />
              </Button>

              {getPageRange(currentPage, totalPages).map((page, index) => (
                <Fragment key={index}>
                  {page === '...' ? (
                    <span>{page}</span>
                  ) : (
                    <Button
                      size="small"
                      variant={page === currentPage ? 'primary' : 'tertiary'}
                      title={`Page ${page}`}
                      onClick={() => setPagination({ ...pagination, pageIndex: Number(page) - 1 })}
                    >
                      {page}
                    </Button>
                  )}
                </Fragment>
              ))}

              <Button
                onClick={handleNextPage}
                disabled={!canNextPage}
                title="Next Page"
                size="small"
                variant="tertiary"
              >
                <ChevronRightIcon size={16} />
              </Button>
            </Flex>
            <Flex direction="row" align="center" gap={spaces.space6}>
              <CountInfo>
                Showing <span>{from}</span> to <span>{to}</span> of <span>{totalCount}</span>{' '}
                results
              </CountInfo>

              <Select
                value={String(pageSize)}
                onValueChange={value =>
                  setPagination({ ...pagination, pageSize: Number(value), pageIndex: 0 })
                }
                placeholder="Select page limit"
                size="small"
              >
                {[10, 25, 50, 100].map(pageSize => (
                  <SelectItem key={pageSize} value={String(pageSize)}>
                    Show {pageSize}
                  </SelectItem>
                ))}
              </Select>
            </Flex>
          </Fragment>
        ) : (
          <Fragment>
            <Skeleton width={300} height={24} />
            <Skeleton width={300} height={24} />
          </Fragment>
        )}
      </Flex>
    </nav>
  )
}

export default Pagination
