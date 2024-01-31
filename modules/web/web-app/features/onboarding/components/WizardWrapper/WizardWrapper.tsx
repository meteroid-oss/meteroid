import { StyledWizardWrapper } from './WizardWrapper.styled'

import type { FunctionComponent, ReactNode } from 'react'

interface WizardWrapperProps {
  children?: ReactNode
}

const WizardWrapper: FunctionComponent<WizardWrapperProps> = ({ children }) => {
  return <StyledWizardWrapper>{children}</StyledWizardWrapper>
}

export default WizardWrapper
