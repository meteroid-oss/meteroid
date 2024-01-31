import { Items, Label, StyledGroup } from '@ui/components/SidebarMenu/components/Group/Group.styled'
import Item from '@ui/components/SidebarMenu/components/Item'
import { ItemProps } from '@ui/components/SidebarMenu/components/Item/Item.types'

import type { FunctionComponent } from 'react'

export interface GroupProps {
  label: string
  items: ItemProps[]
}

const Group: FunctionComponent<GroupProps> = ({ label, items }) => {
  return (
    <StyledGroup>
      <Label>{label}</Label>
      <Items>
        {items.map((item, index) => (
          <Item key={index} {...item} />
        ))}
      </Items>
    </StyledGroup>
  )
}

export default Group
