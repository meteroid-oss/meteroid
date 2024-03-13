import { ColumnDef, OnChangeFn, PaginationState } from '@tanstack/react-table'
import { MoreVerticalIcon } from 'lucide-react'
import { useMemo } from 'react'
import { Link } from 'react-router-dom'

import { StandardTable } from '@/components/table/StandardTable'
import { StatusPill } from '@/features/invoices/StatusPill'
import { amountFormat } from '@/features/invoices/amountFormat'
import { Invoice, InvoiceStatus, InvoicingProvider } from '@/rpc/api/invoices/v1/models_pb'
import {
  PlainMessage,
  BinaryReadOptions,
  JsonValue,
  JsonReadOptions,
  BinaryWriteOptions,
  JsonWriteOptions,
  JsonWriteStringOptions,
  MessageType,
} from '@bufbuild/protobuf'
import { Popover, PopoverContent, PopoverTrigger } from '@md/ui'

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
        header: 'Customer',
        cell: ({ row }) => (
          <Link to={`${linkPrefix}${row.original.id}`}>{row.original.customerName}</Link>
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

          // <Dropdown
          //   side="bottom"
          //   align="start"
          //   overlay={
          //     <div className="pl-4">
          //       <Link to={`${linkPrefix}${row.original.id}`}>
          //         <Dropdown.Item key="header" className="hover:bg-slate-500">
          //           View invoice
          //         </Dropdown.Item>
          //       </Link>
          //     </div>
          //   }
          // >
          //   <MoreVerticalIcon size={16} className="cursor-pointer" />
          // </Dropdown>
        ),
      },
    ],
    []
  )

  type InvoiceLike = {
    [key in keyof Invoice]: Invoice[key]
  }

  const t: InvoiceLike = {
    id: 'dsqd',
    status: InvoiceStatus.DRAFT,
    invoicingProvider: InvoicingProvider.STRIPE,
    invoiceDate: '',
    customerId: '',
    customerName: '',
    subscriptionId: '',
    currency: '',
    equals: function (other: Invoice | PlainMessage<Invoice> | null | undefined): boolean {
      throw new Error('Function not implemented.')
    },
    clone: function (): Invoice {
      throw new Error('Function not implemented.')
    },
    fromBinary: function (
      bytes: Uint8Array,
      options?: Partial<BinaryReadOptions> | undefined
    ): Invoice {
      throw new Error('Function not implemented.')
    },
    fromJson: function (
      jsonValue: JsonValue,
      options?: Partial<JsonReadOptions> | undefined
    ): Invoice {
      throw new Error('Function not implemented.')
    },
    fromJsonString: function (
      jsonString: string,
      options?: Partial<JsonReadOptions> | undefined
    ): Invoice {
      throw new Error('Function not implemented.')
    },
    toBinary: function (options?: Partial<BinaryWriteOptions> | undefined): Uint8Array {
      throw new Error('Function not implemented.')
    },
    toJson: function (options?: Partial<JsonWriteOptions> | undefined): JsonValue {
      throw new Error('Function not implemented.')
    },
    toJsonString: function (options?: Partial<JsonWriteStringOptions> | undefined): string {
      throw new Error('Function not implemented.')
    },
    getType: function (): MessageType<Invoice> {
      throw new Error('Function not implemented.')
    },
  }

  return (
    <StandardTable
      columns={columns}
      data={[t as Invoice]}
      sortable={true}
      pagination={pagination}
      setPagination={setPagination}
      totalCount={totalCount}
      isLoading={isLoading}
    />
  )
}
