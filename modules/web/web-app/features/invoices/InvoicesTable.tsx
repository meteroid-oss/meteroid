import { G } from '@mobily/ts-belt'
import { ColumnDef, OnChangeFn, PaginationState } from '@tanstack/react-table'
import { Dropdown } from '@ui/components'
import { MoreVerticalIcon } from 'lucide-react'
import { useMemo } from 'react'
import { Link } from 'react-router-dom'

import { StandardTable } from '@/components/table/StandardTable'
import { StatusPill } from '@/features/invoices/StatusPill'
import { Invoice } from '@/rpc/api/invoices/v1/models_pb'

import type { FunctionComponent } from 'react'

interface CustomersTableProps {
  data: Invoice[]
  pagination: PaginationState
  setPagination: OnChangeFn<PaginationState>
  totalCount: number
  isLoading?: boolean
}

export const InvoicesTable: FunctionComponent<CustomersTableProps> = ({
  data,
  pagination,
  setPagination,
  totalCount,
  isLoading,
}) => {
  const columns = useMemo<ColumnDef<Invoice>[]>(
    () => [
      {
        header: 'Customer',
        cell: ({ row }) => <Link to={`${row.original.id}`}>{row.original.customerName}</Link>,
      },
      {
        header: 'Amount',
        accessorFn: cell => !G.isNullable(cell.amountCents)
          ? new Intl.NumberFormat(navigator.language).format(cell.amountCents)
          : '',
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
          <Dropdown
            side="bottom"
            align="start"
            overlay={
              <div className="pl-4">
                <Link to={`${row.original.id}`}>
                  <Dropdown.Item key="header" className="hover:bg-slate-500">
                    View invoice
                  </Dropdown.Item>
                </Link>
              </div>
            }
          >
            <MoreVerticalIcon size={16} className="cursor-pointer" />
          </Dropdown>
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
