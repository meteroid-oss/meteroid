import Group from './components/Group'
import Item, { ItemProps } from './components/Item'

import type { FunctionComponent } from 'react'

export interface MenuProps {
  items: MenuItemsProps[]
}

export type MenuItemsProps =
  | {
      label: string
      items: ItemProps[]
    }
  | ItemProps

const SidebarMenuComponent: FunctionComponent<MenuProps> = ({ items }) => {
  return (
    <div className="flex flex-col space-y-8 overflow-y-auto">
      <nav className="px-4">
        {items.map(({ ...props }, index) =>
          'items' in props ? (
            <Group key={index} label={props.label} items={props.items} />
          ) : (
            <Item key={index} {...props} />
          )
        )}
      </nav>
    </div>
  )
}

export default SidebarMenuComponent
