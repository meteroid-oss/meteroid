import { FunctionComponent } from 'react'

import { CatalogHeader } from '@/features/productCatalog/generic/CatalogHeader'

interface ProductsHeaderProps {
  heading: string
  isLoading: boolean
  refetch: () => void
  setEditPanelVisible: (visible: boolean) => void
  setSearch: (value: string | undefined) => void
}

export const ProductsHeader: FunctionComponent<ProductsHeaderProps> = props => {
  return <CatalogHeader {...props} newButtonText="New product" />
}
