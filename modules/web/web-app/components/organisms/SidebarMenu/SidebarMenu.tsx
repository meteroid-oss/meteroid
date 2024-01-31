import { SidebarMenu } from '@md/ui'

import FamilyPicker from '@/components/atoms/FamilyPicker/FamilyPicker'
import { MenuProps } from '@/components/organisms/SidebarMenu/SidebarMenu.types'

import type { FunctionComponent } from 'react'

const SidebarMenuComponent: FunctionComponent<MenuProps> = ({ items }) => {
  return (
    <div className="flex flex-col space-y-8 overflow-y-auto">
      <SidebarMenu>
        <FamilyPicker />
        {items.map(({ label, items }, index) => (
          <SidebarMenu.Group key={index} label={label} items={items} />
        ))}
      </SidebarMenu>
    </div>
  )
}

export default SidebarMenuComponent
