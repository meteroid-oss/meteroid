import { ColumnDef, OnChangeFn, PaginationState } from '@tanstack/react-table'
import { MoreVerticalIcon } from 'lucide-react'
import { useMemo } from 'react'

import { StandardTable } from '@/components/table/StandardTable'
import { Customer } from '@/rpc/api/customers/v1/models_pb'

import type { FunctionComponent } from 'react'

interface CustomersTableProps {
  data: Customer[]
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
  const columns = useMemo<ColumnDef<Customer>[]>(
    () => [
      {
        header: 'Name',
        accessorKey: 'name',
      },
      {
        header: 'Active subscriptions',
        accessorFn: () => '-',
      },
      {
        header: 'Alias',
        accessorFn: cell => cell.alias, // TODO get only the count from db ?
      },
      {
        header: 'Accrued',
        accessorFn: () => '0$',
      },
      {
        accessorKey: 'id',
        header: '',
        className: 'w-2',
        cell: () => <MoreVerticalIcon size={16} className="cursor-pointer" />,
      },
    ],
    []
  )

  return (
    <StandardTable
      columns={columns}
      data={data}
      sortable={true}
      pagination={pagination}
      setPagination={setPagination}
      totalCount={totalCount}
      isLoading={isLoading}
    />
  )
}
