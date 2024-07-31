import { Popover, PopoverContent, PopoverTrigger } from '@md/ui'
import { ColumnDef, OnChangeFn, PaginationState } from '@tanstack/react-table'
import { MoreVerticalIcon } from 'lucide-react'
import { useMemo } from 'react'
import { Link } from 'react-router-dom'

import { StandardTable } from '@/components/table/StandardTable'
import { StatusPill } from '@/features/invoices/StatusPill'
import { amountFormat } from '@/features/invoices/amountFormat'
import { Invoice } from '@/rpc/api/invoices/v1/models_pb'

interface CustomersTableProps {
  data: Invoice[]
  pagination: PaginationState
  setPagination: OnChangeFn<PaginationState>
  totalCount: number
  isLoading?: boolean
  linkPrefix?: string
}

export const InvoicesTable = ({
  data,
  pagination,
  setPagination,
  totalCount,
  isLoading,
  linkPrefix = '',
}: CustomersTableProps) => {
  const columns = useMemo<ColumnDef<Invoice>[]>(
    () => [
      {
        header: 'Invoice Number',
        cell: ({ row }) => (
          <Link to={`${linkPrefix}${row.original.id}`}>{row.original.invoiceNumber}</Link>
        ),
      },
      {
        header: 'Customer',
        cell: ({ row }) => (
          <Link to={`../../customers/${row.original.customerId}`}>{row.original.customerName}</Link>
        ),
      },
      {
        header: 'Amount',
        accessorFn: amountFormat,
      },
      {
        header: 'Currency',
        accessorKey: 'currency',
      },
      {
        header: 'Invoice date',
        accessorFn: cell => cell.invoiceDate,
      },
      {
        header: 'Status',
        cell: ({ row }) => <StatusPill status={row.original.status} />,
      },
      {
        accessorKey: 'id',
        header: '',
        className: 'w-2',
        cell: ({ row }) => (
          <Popover>
            <PopoverTrigger>
              <MoreVerticalIcon size={16} className="cursor-pointer" />
            </PopoverTrigger>
            <PopoverContent className="p-0 pl-4 text-sm w-36 " side="bottom" align="end">
              <Link
                className="flex items-center h-10 w-full "
                to={`${linkPrefix}${row.original.id}`}
              >
                View invoice
              </Link>
            </PopoverContent>
          </Popover>
        ),
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
