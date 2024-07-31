import { ColumnDef, OnChangeFn, PaginationState } from '@tanstack/react-table'
import { MoreVerticalIcon } from 'lucide-react'
import { useMemo } from 'react'
import { Link } from 'react-router-dom'

import { StandardTable } from '@/components/table/StandardTable'
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
  const columns = useMemo<ColumnDef<CustomerBrief>[]>(
    () => [
      {
        header: 'Name',
        cell: ({ row }) => <Link to={`${row.original.id}`}>{row.original.name}</Link>,
      },
      {
        header: 'Country',
        cell: ({ row }) => {
          row.original.country
        },
      },
      {
        header: 'Email',
        cell: ({ row }) => {
          row.original.email
        },
      },
      {
        header: 'Alias',
        accessorFn: cell => cell.alias, // TODO get only the count from db ?
      },
      {
        header: 'Accrued',
        accessorFn: () => '-', // TODO get only the count from db ?
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
