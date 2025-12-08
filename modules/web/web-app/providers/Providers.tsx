import { TooltipProvider } from '@md/ui'
import { Toaster } from '@ui/components/ui/sonner'
import { Outlet } from 'react-router-dom'

import ConfirmationModalProvider from 'providers/ConfirmationProvider'
import { ThemeProvider, useTheme } from 'providers/ThemeProvider'

export const Providers: React.FC = () => {
  return (
    <TooltipProvider>
      <ThemeProvider>
        {/* <FlagsProvider> */}
        <ConfirmationModalProvider>
          <Outlet />
        </ConfirmationModalProvider>
        {/* </FlagsProvider> */}
        <ToasterWithTheme />
      </ThemeProvider>
    </TooltipProvider>
  )
}

const ToasterWithTheme = () => {
  const theme = useTheme()
  return <Toaster theme={theme.isDarkMode ? 'dark' : 'light'} />
}
