import { FunctionComponent } from 'react'

import { CatalogHeader } from '@/features/productCatalog/generic/CatalogHeader'

interface ProductItemsHeaderProps {
  heading: string
  isLoading: boolean
  refetch: () => void
  setEditPanelVisible: (visible: boolean) => void
  setSearch: (value: string | undefined) => void
}

export const ProductItemsHeader: FunctionComponent<ProductItemsHeaderProps> = props => {
  return <CatalogHeader {...props} newButtonText="New product" />
}
