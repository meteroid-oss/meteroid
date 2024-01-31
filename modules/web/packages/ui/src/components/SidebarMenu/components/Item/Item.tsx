import { NavLink } from 'react-router-dom'

import { ItemLink, StyledItem } from '@ui/components/SidebarMenu/components/Item/Item.styled'
import { ItemProps } from '@ui/components/SidebarMenu/components/Item/Item.types'

import type { FunctionComponent } from 'react'

const Item: FunctionComponent<ItemProps> = ({ label, to, end }) => {
  return (
    <StyledItem>
      <NavLink to={to} end={end}>
        {({ isActive }) => <ItemLink isActive={isActive}>{label}</ItemLink>}
      </NavLink>
    </StyledItem>
  )
}

export default Item
