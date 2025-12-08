import { LogoSymbol } from '@md/ui'

import { useTheme } from 'providers/ThemeProvider'

import type { FunctionComponent } from 'react'

interface PageLoaderProps {
  title?: string
}

const PageLoader: FunctionComponent<PageLoaderProps> = ({ title }) => {
  const { isDarkMode } = useTheme()
  return (
    <div className="flex justify-center items-center h-screen w-screen fixed top-0 left-0 bg-background z-50 text-muted-foreground text-sm fadeIn">
      <div className="flex flex-col gap-1.5 items-center justify-center">
        <div className="animate-bounce-logo">
          <LogoSymbol isDarkMode={isDarkMode} size="large" />
        </div>
        <h1>{title}</h1>
      </div>
    </div>
  )
}

export default PageLoader
