import { StyledStepTitle } from './StepTitle.styled'

import type { FunctionComponent, ReactNode } from 'react'

interface StepTitleProps {
  children: ReactNode
}

const StepTitle: FunctionComponent<StepTitleProps> = ({ children }) => {
  return <StyledStepTitle>{children}</StyledStepTitle>
}

export default StepTitle
