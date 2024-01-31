import { To } from 'react-router-dom'

export interface ProductMenuGroup {
  title?: string
  isPreview?: boolean
  items: ProductMenuGroupItem[]
}

export interface ProductMenuGroupItem {
  to: To
  label: string
  icon?: JSX.Element
  end?: boolean
}
