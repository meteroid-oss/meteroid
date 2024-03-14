import { TooltipProvider } from '@md/ui'
import { Outlet } from 'react-router-dom'

import ConfirmationModalProvider from 'providers/ConfirmationProvider'
import { ThemeProvider } from 'providers/ThemeProvider'

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
