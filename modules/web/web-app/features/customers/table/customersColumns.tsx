import { ColumnDef } from '@tanstack/react-table'
import { CountryFlag } from '@ui/components'
import { MoreVerticalIcon } from 'lucide-react'
import { useMemo } from 'react'
import { Link } from 'react-router-dom'

import { CustomerBrief } from '@/rpc/api/customers/v1/models_pb'

export const useCustomersColumns = () =>
  useMemo<ColumnDef<CustomerBrief>[]>(
    () => [
      {
        header: 'Name',
        cell: ({ row }) => <Link to={`${row.original.id}`}>{row.original.name}</Link>,
      },
      {
        header: 'Country',
        cell: ({ row }) => <CountryFlag name={row.original.country} />,
      },
      {
        header: 'Email',
        cell: ({ row }) => {
          row.original.billingEmail
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
