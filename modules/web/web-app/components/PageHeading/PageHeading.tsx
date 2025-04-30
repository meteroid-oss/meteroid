import { Count, StyledPageHeading } from '@/components/PageHeading/PageHeading.styled'

import type { FunctionComponent, ReactNode } from 'react'

interface PageHeadingProps {
  children: ReactNode
  count?: number
}

const PageHeading: FunctionComponent<PageHeadingProps> = ({ children, count }) => {
  return (
    <StyledPageHeading>
      {children}
      {count !== undefined && count >= 0 && <Count>yo({count})</Count>}
    </StyledPageHeading>
  )
}

export default PageHeading
