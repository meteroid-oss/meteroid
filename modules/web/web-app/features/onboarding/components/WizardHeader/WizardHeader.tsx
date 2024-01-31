import { spaces } from '@md/foundation'
import { Flex, useWizard } from '@ui/components'
import { Link } from 'react-router-dom'

import { useTheme } from 'providers/ThemeProvider'

import { Item, Items, Logo, StepCount, StyledWizard } from './WizardHeader.styled'

import type { FunctionComponent } from 'react'

const WizardHeader: FunctionComponent = () => {
  const { stepCount, activeStep } = useWizard()
  const { isDarkMode } = useTheme()

  return (
    <StyledWizard>
      <Flex direction="column" align="center" gap={spaces.space12}>
        <Items>
          {Array.from({ length: stepCount }).map((_, index) => (
            <Item key={index} tabIndex={index} active={activeStep === index} />
          ))}
        </Items>
        <StepCount>
          Step {activeStep + 1} of {stepCount}
        </StepCount>
      </Flex>
      <Link to="/">
        <Logo isDarkMode={isDarkMode} />
      </Link>
    </StyledWizard>
  )
}

export default WizardHeader
