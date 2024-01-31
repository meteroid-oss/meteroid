import { LogoSymbol } from '@md/foundation'
import { Link } from 'react-router-dom'

import { useTheme } from 'providers/ThemeProvider'

import { StyledHeader } from './Header.styled'

import type { FunctionComponent } from 'react'

const Header: FunctionComponent = () => {
  const { isDarkMode } = useTheme()

  return (
    <StyledHeader>
      <Link to=".">
        <LogoSymbol isDarkMode={isDarkMode} />
      </Link>
    </StyledHeader>
  )
}

export default Header
