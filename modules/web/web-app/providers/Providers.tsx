import { TooltipProvider } from '@md/ui'
import { Outlet } from 'react-router-dom'

import { ThemeProvider } from 'providers/ThemeProvider'

export const Providers: React.FC = () => {
  return (
    <TooltipProvider>
      <ThemeProvider>
        {/* <FlagsProvider> */}
        <Outlet />
        {/* </FlagsProvider> */}
      </ThemeProvider>
    </TooltipProvider>
  )
}
