import { TooltipProvider } from '@ui2/components'
import { Outlet } from 'react-router-dom'

import { ThemeProvider } from 'providers/ThemeProvider'
import ConfirmationModalProvider from 'providers/ConfirmationProvider'

export const Providers: React.FC = () => {
  return (
    <TooltipProvider>
      <ThemeProvider>
        {/* <FlagsProvider> */}
        <ConfirmationModalProvider>
          <Outlet />
        </ConfirmationModalProvider>
        {/* </FlagsProvider> */}
      </ThemeProvider>
    </TooltipProvider>
  )
}
