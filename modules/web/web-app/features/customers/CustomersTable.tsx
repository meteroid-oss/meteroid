import { ColumnDef, OnChangeFn, PaginationState } from '@tanstack/react-table'
import { MoreVerticalIcon } from 'lucide-react'
import { useMemo } from 'react'
import { Link } from 'react-router-dom'

import { StandardTable } from '@/components/table/StandardTable'
import { CustomerList } from '@/rpc/api/customers/v1/models_pb'

import type { FunctionComponent } from 'react'

interface CustomersTableProps {
  data: CustomerList[]
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
  const columns = useMemo<ColumnDef<CustomerList>[]>(
    () => [
      {
        header: 'Name',
        cell: ({ row }) => <Link to={`${row.original.id}`}>{row.original.name}</Link>,
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
