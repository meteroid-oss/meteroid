import Group from './components/Group'
import { ItemProps } from './components/Item'

import type { FunctionComponent } from 'react'

export interface MenuProps {
  items: MenuItemsProps[]
}

export interface MenuItemsProps {
  label: string
  items: ItemProps[]
}

const SidebarMenuComponent: FunctionComponent<MenuProps> = ({ items }) => {
  return (
    <div className="flex flex-col space-y-8 overflow-y-auto">
      <nav className="px-4">
        {items.map(({ label, items }, index) => (
          <Group key={index} label={label} items={items} />
        ))}
      </nav>
    </div>
  )
}

export default SidebarMenuComponent
