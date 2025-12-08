import { LogoSymbol } from '@md/ui'
import { Link } from 'react-router-dom'

import { useTheme } from 'providers/ThemeProvider'

import type { FunctionComponent } from 'react'

const Header: FunctionComponent = () => {
  const { isDarkMode } = useTheme()

  return (
    <header>
      <Link to=".">
        <LogoSymbol isDarkMode={isDarkMode} />
      </Link>
    </header>
  )
}

export default Header
