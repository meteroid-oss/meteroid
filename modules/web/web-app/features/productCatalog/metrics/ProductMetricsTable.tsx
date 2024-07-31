import { ColumnDef, OnChangeFn, PaginationState } from '@tanstack/react-table'
import { MoreVerticalIcon } from 'lucide-react'
import { FC, useMemo } from 'react'

import { StandardTable } from '@/components/table/StandardTable'
import {
  Aggregation_AggregationType,
  BillableMetricMeta,
} from '@/rpc/api/billablemetrics/v1/models_pb'

const aggregationTypeMapper: Record<Aggregation_AggregationType, string> = {
  [Aggregation_AggregationType.SUM]: 'sum',
  [Aggregation_AggregationType.MIN]: 'min',
  [Aggregation_AggregationType.MAX]: 'max',
  [Aggregation_AggregationType.MEAN]: 'mean',
  [Aggregation_AggregationType.LATEST]: 'latest',
  [Aggregation_AggregationType.COUNT]: 'count',
  [Aggregation_AggregationType.COUNT_DISTINCT]: 'distinct',
}
interface BillableMetricableProps {
  data: BillableMetricMeta[]
  pagination: PaginationState
  setPagination: OnChangeFn<PaginationState>
  totalCount: number
}
export const BillableMetricTable: FC<BillableMetricableProps> = ({
  data,
  pagination,
  setPagination,
  totalCount,
}) => {
  console.log(data)
  const columns = useMemo<ColumnDef<BillableMetricMeta>[]>(
    () => [
      {
        header: 'Name',
        accessorKey: 'name',
      },

      {
        header: 'Description',
        accessorKey: 'description',
      },
      {
        header: 'Event name',
        accessorKey: 'code',
      },
      {
        header: 'Aggregation',
        maxSize: 0.1,
        cell: c => (
          <code>
            {aggregationTypeMapper[c.row.original.aggregationType]}
            {c.row.original.aggregationKey && <>({c.row.original.aggregationKey})</>}
          </code>
        ),
      },
      {
        header: 'Plans',
        accessorFn: () => '0',
      },

      {
        accessorKey: 'id',
        header: '',
        maxSize: 0.1,
        cell: () => <MoreVerticalIcon size={16} className="cursor-pointer" />,
      },
    ],
    []
  )

  return (
    <StandardTable
      columns={columns}
      data={data}
      pagination={pagination}
      setPagination={setPagination}
      totalCount={totalCount}
    />
  )
}
