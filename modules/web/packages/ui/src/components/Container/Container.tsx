import { FunctionComponent, ReactNode } from 'react'

import { StyledContainer } from './Container.styled'

interface ContainerProps {
  children: ReactNode
  fullHeight?: boolean
}

export const Container: FunctionComponent<ContainerProps> = ({ children, fullHeight }) => {
  return <StyledContainer fullHeight={fullHeight}>{children}</StyledContainer>
}
