import { LogoSymbol, spaces } from '@md/foundation'
import { Flex } from '@ui2/components/legacy'

import { useTheme } from 'providers/ThemeProvider'

import { AnimatedLogo, StyledPageLoader } from './PageLoader.styled'

import type { FunctionComponent } from 'react'

interface PageLoaderProps {
  title?: string
}

const PageLoader: FunctionComponent<PageLoaderProps> = ({ title }) => {
  const { isDarkMode } = useTheme()
  return (
    <StyledPageLoader>
      <Flex direction="column" gap={spaces.space3} align="center" justify="center">
        <AnimatedLogo>
          <LogoSymbol isDarkMode={isDarkMode} size="large" />
        </AnimatedLogo>
        <h1>{title}</h1>
      </Flex>
    </StyledPageLoader>
  )
}

export default PageLoader
