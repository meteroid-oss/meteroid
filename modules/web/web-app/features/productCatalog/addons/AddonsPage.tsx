// import { spaces } from '@md/foundation'
// import { Flex } from '@ui/components/legacy'
import { FunctionComponent } from 'react'

// import { ProductItemsTable } from '@/features/productCatalog/items/ProductItemsTable'
// import { useQuery } from '@/lib/connectrpc'
// import { listAddOns } from '@/rpc/api/addons/v1/addons-AddOnsService_connectquery'
// // import { ListAddOnRequest_SortBy } from '@/rpc/api/addons/v1/addons_pb'

// import { AddonCreatePanel } from '@/features/productCatalog/addons/AddonCreatePanel'
// import { CatalogHeader } from '@/features/productCatalog/generic/CatalogHeader'
// import { useCatalogPageProps } from '@/features/productCatalog/generic/useCatalogPageProps'
// import { useQueryClient } from '@tanstack/react-query'

export const AddonsPage: FunctionComponent = () => {
  return <>Not implemented</>

  // const { baseQuery, paginationState, onSearch } = useCatalogPageProps()
  // const [editPanelVisible, setEditPanelVisible] = useState(false)

  // const query = useQuery(
  //   listAddOns
  // TODO
  // baseQuery
  //   ? {
  //       ...baseQuery,
  //       // sortBy: ListAddOnRequest_SortBy.DATE_DESC,
  //     }
  //   : disableQuery
  // )

  // const queryClient = useQueryClient()

  // return (
  //   <Flex direction="column" gap={spaces.space9}>
  //     <CatalogHeader
  //       heading="Addonsz"
  //       newButtonText="New addonz"
  //       setEditPanelVisible={setEditPanelVisible}
  //       isLoading={query.isLoading}
  //       refetch={query.refetch}
  //       setSearch={onSearch}
  //     />
  //     <ProductItemsTable
  //       data={query.data?.addOns ?? []}
  //       pagination={paginationState[0]}
  //       setPagination={paginationState[1]}
  //       totalCount={/*TODO query.data?.paginationMeta?.total ??*/ 0}
  //       isLoading={query.isLoading}
  //     />
  //     {/* TODO route-based, edit etc */}
  //     <AddonCreatePanel />
  //   </Flex>
  // )
}
