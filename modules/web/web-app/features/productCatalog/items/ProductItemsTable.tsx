import { ColumnDef, PaginationState } from '@tanstack/react-table'
import { MoreVerticalIcon } from 'lucide-react'
import { FC, useMemo } from 'react'

import { expandColumn } from '@/components/table/ExpandableTable'
import { StandardTable } from '@/components/table/StandardTable'
import { Product } from '@/rpc/api/products/v1/models_pb'

import type { OnChangeFn } from '@tanstack/react-table'

type ProductItem = Product & {
  isExpandable?: boolean | undefined
}

interface ProductItemsTableProps {
  data: ProductItem[]
  pagination: PaginationState
  setPagination: OnChangeFn<PaginationState>
  totalCount: number
  isLoading?: boolean
}
export const ProductItemsTable: FC<ProductItemsTableProps> = ({
  data,
  pagination,
  setPagination,
  totalCount,
  isLoading,
}) => {
  const columns = useMemo<ColumnDef<ProductItem>[]>(
    () => [
      expandColumn as ColumnDef<ProductItem>,
      {
        header: 'Name',
        accessorKey: 'name',
      },
      {
        header: 'Description',
        accessorKey: 'description',
      },
      {
        header: 'Created at',
        accessorFn: cell => cell.createdAt?.toDate().toLocaleDateString(),
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
    <>
      {/* // TODO MERGE  */}
      <StandardTable
        columns={columns}
        data={data}
        sortable={true}
        pagination={pagination}
        setPagination={setPagination}
        totalCount={totalCount}
        isLoading={isLoading}
      />
      {/* // TODO MERGE MID */}
      {/* <div className="justify-between px-6 pt-6 pb-2 md:flex">
        <div className="relative flex space-x-4">
          <Input
            size="small"
            placeholder="Search"
            // value={"filterString"}
            // onChange={(e: any) => setFilterString(e.target.value)}
            icon={<SearchIcon size={14} />}
          />
        </div>
        <div className="mt-4 flex items-center gap-2 md:mt-0">
          <Button
            size="small"
            icon={<RefreshCwIcon size={14} />}
            type="outline"
            loading={productsQuery.isLoading}
            onClick={() => productsQuery.refetch()}
          />

          <Button
            type="primary"
            size="small"
            icon={<PlusIcon size={14} strokeWidth={1.5} />}
            onClick={() => setEditPanelVisible(true)}
          >
            New product
          </Button>
        </div>
      </div>
      <section className="thin-scrollbars mt-4 overflow-visible px-6">
        <div className="section-block--body relative overflow-x-auto rounded">
          <div className="inline-block min-w-full align-middle">
            <ExpandableTable
              columns={columns}
              data={productsQuery.data as ProductItem[]}
              pagination={pagination}
              setPagination={setPagination}
              totalCount={totalCount}
              renderSubComponent={({ row }) => <SubComponent item={row.original} />}
            />
          </div>
        </div>
      </section> */}
    </>
  )
}

// const SubComponent = ({ item }: { item: ProductItem }) => {
//   const details = trpc.catalog.products.details.useQuery({
//     id: item.id,
//   })
//   if (details.isLoading) return <Loading />
//   if (details.error) return <>Error: {details.error.message}</>
//   return (
//     <>
//       <div className="my-2 space-y-2 px-5">
//         <h5 className="text-slate-1200 flex space-x-1">
//           <DollarSignIcon size={16} />
//           <span>Fixed charges</span>
//         </h5>
//         <div className="text-sm text-muted-foreground">
//           {details.data?.charges.map((charge, idx) => (
//             <div key={`charge-${idx}`} className="space-x-1 ml-5 flex">
//               - <div>{charge.name}</div> <div>{charge.description}</div>
//             </div>
//           ))}
//         </div>
//         <h5 className="text-slate-1200 flex space-x-1">
//           <GaugeIcon size={16} />
//           <span>Metered charges</span>
//         </h5>
//         <div className="text-sm text-slate-900">
//           {details.data?.billableMetrics.map((charge, idx) => (
//             <div key={`charge-${idx}`} className="space-x-1 ml-5 flex">
//               <div>- {charge.name}</div> <div>{charge.description}</div>
//             </div>
//           ))}
//         </div>
//       </div>
//     </>
//   )
// }
