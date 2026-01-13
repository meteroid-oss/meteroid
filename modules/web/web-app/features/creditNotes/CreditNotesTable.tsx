import { Popover, PopoverContent, PopoverTrigger } from '@md/ui'
import { ColumnDef, OnChangeFn, PaginationState } from '@tanstack/react-table'
import { MoreVerticalIcon } from 'lucide-react'
import { useMemo } from 'react'
import { Link } from 'react-router-dom'

import { StandardTable } from '@/components/table/StandardTable'
import { CreditNoteStatusPill } from '@/features/creditNotes/CreditNoteStatusPill'
import { useBasePath } from '@/hooks/useBasePath'
import { formatCurrency } from '@/lib/utils/numbers'
import { CreditNote } from '@/rpc/api/creditnotes/v1/models_pb'
import { parseAndFormatDate } from '@/utils/date'

interface CreditNotesTableProps {
  data: CreditNote[]
  pagination: PaginationState
  setPagination: OnChangeFn<PaginationState>
  totalCount: number
  isLoading?: boolean
}

export const CreditNotesTable = ({
  data,
  pagination,
  setPagination,
  totalCount,
  isLoading,
}: CreditNotesTableProps) => {
  const basePath = useBasePath()

  const columns = useMemo<ColumnDef<CreditNote>[]>(
    () => [
      {
        header: 'Credit Note #',
        accessorKey: 'creditNoteNumber',
      },
      {
        header: 'Customer',
        accessorKey: 'customerName',
      },
      {
        header: 'Amount',
        accessorFn: cell => formatCurrency(Math.abs(Number(cell.total)), cell.currency),
      },
      {
        header: 'Currency',
        accessorKey: 'currency',
      },
      {
        header: 'Created',
        accessorFn: cell => parseAndFormatDate(cell.createdAt),
      },
      {
        header: 'Status',
        cell: ({ row }) => <CreditNoteStatusPill status={row.original.status} />,
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
                to={`${basePath}/credit-notes/${row.original.id}`}
              >
                View credit note
              </Link>
            </PopoverContent>
          </Popover>
        ),
      },
    ],
    [basePath]
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
      rowLink={row => `${basePath}/credit-notes/${row.original.id}`}
    />
  )
}
