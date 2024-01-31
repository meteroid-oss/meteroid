import { ItemProps } from '@ui/components/SidebarMenu/components/Item/Item.types'

export interface MenuProps {
  items: MenuItemsProps[]
}

export interface MenuItemsProps {
  label: string
  items: ItemProps[]
}
