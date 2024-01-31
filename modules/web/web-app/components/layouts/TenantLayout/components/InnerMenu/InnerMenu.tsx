import { Header, HeaderTitle, StyledInnerMenu } from './InnerMenu.styled'

import type { FunctionComponent, ReactNode } from 'react'

interface InnerMenuProps {
  title: string
  children: ReactNode
}

const InnerMenu: FunctionComponent<InnerMenuProps> = ({ title, children }) => {
  return (
    <StyledInnerMenu>
      <Header>
        <HeaderTitle>{title}</HeaderTitle>
      </Header>

      {children}
    </StyledInnerMenu>
  )
}

export default InnerMenu
