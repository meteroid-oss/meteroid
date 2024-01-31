import { StyledSidebarMenu } from '@ui/components/SidebarMenu/SidebarMenu.styled'

import Group from './components/Group'
import Item from './components/Item'

import type { ReactNode } from 'react'

interface SidebarMenuProps {
  children: ReactNode
}

function SidebarMenu({ children }: SidebarMenuProps) {
  return <StyledSidebarMenu>{children}</StyledSidebarMenu>
}

SidebarMenu.Group = Group
SidebarMenu.Item = Item

export default SidebarMenu
