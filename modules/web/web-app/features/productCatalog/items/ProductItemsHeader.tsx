import { FunctionComponent } from 'react'

import { CatalogHeader } from '@/features/productCatalog/generic/CatalogHeader'

interface ProductsHeaderProps {
  heading: string
  count?: number
  isLoading: boolean
  refetch: () => void
  setEditPanelVisible: (visible: boolean) => void
  setSearch: (value: string | undefined) => void
}

export const ProductsHeader: FunctionComponent<ProductsHeaderProps> = props => {
  return <CatalogHeader {...props} />
}
