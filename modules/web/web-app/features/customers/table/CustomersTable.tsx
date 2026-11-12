import { OnChangeFn, PaginationState, Row, SortingState } from '@tanstack/react-table'
import { ReactNode, useMemo } from 'react'

import { ColumnState, StandardTable } from '@/components/table/StandardTable'
import { useCustomersColumns } from '@/features/customers/table/customersColumns'
import {
  useCustomPropertyColumns,
  withCustomPropertyColumns,
} from '@/features/customproperties/useCustomPropertyColumns'
import { useBasePath } from '@/hooks/useBasePath'
import { usePrefetch } from '@/hooks/usePrefetch'
import { getCustomerById } from '@/rpc/api/customers/v1/customers-CustomersService_connectquery'
import { CustomerBrief } from '@/rpc/api/customers/v1/models_pb'
import { CustomPropertyEntityType } from '@/rpc/api/customproperties/v1/models_pb'

import type { FunctionComponent } from 'react'

interface CustomersTableProps {
  data: CustomerBrief[]
  pagination: PaginationState
  setPagination: OnChangeFn<PaginationState>
  totalCount: number
  isLoading?: boolean
  sorting?: SortingState
  onSortingChange?: OnChangeFn<SortingState>
  /** Column visibility controlled by an active saved view. */
  columnState?: ColumnState
  /** Bulk actions over the selected rows; enables row selection when set. */
  renderSelectionActions?: (rows: Row<CustomerBrief>[]) => ReactNode
}

export const CustomersTable: FunctionComponent<CustomersTableProps> = ({
  data,
  pagination,
  setPagination,
  totalCount,
  isLoading,
  sorting,
  onSortingChange,
  columnState,
  renderSelectionActions,
}) => {
  const basePath = useBasePath()
  const prefetch = usePrefetch()

  const baseColumns = useCustomersColumns({ rows: data })
  const { columns: customPropertyColumns, hiddenIds } = useCustomPropertyColumns<CustomerBrief>(
    CustomPropertyEntityType.CUSTOMER
  )
  const columns = useMemo(
    () => withCustomPropertyColumns(baseColumns, customPropertyColumns),
    [baseColumns, customPropertyColumns]
  )

  return (
    <div className="">
      <StandardTable
        tableKey="customers"
        columnState={columnState}
        columns={columns}
        data={data}
        sortable={true}
        sorting={sorting}
        onSortingChange={onSortingChange}
        pagination={pagination}
        setPagination={setPagination}
        totalCount={totalCount}
        isLoading={isLoading}
        defaultHidden={hiddenIds}
        rowLink={row => `${basePath}/customers/${row.original.id}`}
        onRowIntent={row => prefetch(getCustomerById, { id: row.original.id })}
        enableSelection={!!renderSelectionActions}
        getRowId={row => row.id}
        renderSelectionActions={renderSelectionActions}
      />
    </div>
  )
}
