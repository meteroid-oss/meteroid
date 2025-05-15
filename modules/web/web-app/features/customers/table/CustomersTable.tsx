import { OnChangeFn, PaginationState } from '@tanstack/react-table'

import { StandardTable } from '@/components/table/StandardTable'
import { useCustomersColumns } from '@/features/customers/table/customersColumns'
import { useBasePath } from '@/hooks/useBasePath'
import { CustomerBrief } from '@/rpc/api/customers/v1/models_pb'

import type { FunctionComponent } from 'react'

interface CustomersTableProps {
  data: CustomerBrief[]
  pagination: PaginationState
  setPagination: OnChangeFn<PaginationState>
  totalCount: number
  isLoading?: boolean
}

export const CustomersTable: FunctionComponent<CustomersTableProps> = ({
  data,
  pagination,
  setPagination,
  totalCount,
  isLoading,
}) => {
  const basePath = useBasePath()

  const columns = useCustomersColumns()

  return (
    <div className="">
      <StandardTable
        columns={columns}
        data={data}
        sortable={true}
        pagination={pagination}
        setPagination={setPagination}
        totalCount={totalCount}
        isLoading={isLoading}
        rowLink={row => `${basePath}/customers/${row.original.id}`}
      />
    </div>
  )
}
