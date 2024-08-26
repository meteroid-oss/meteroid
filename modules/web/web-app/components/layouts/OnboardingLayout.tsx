import { MeteroidTitle } from '@/components/svg'
import { useForceTheme } from 'providers/ThemeProvider'
import { Outlet } from 'react-router-dom'

export const OnboardingLayout = () => {
  useForceTheme('light')

  return (
    <div className="flex min-h-full flex-1 flex-col justify-center py-12 sm:px-6 lg:px-8 dark bg-card">
      <div className="sm:mx-auto sm:w-full sm:max-w-[1280px]">
        <MeteroidTitle forceTheme="dark" />
      </div>

      <div className="mt-8 sm:mx-auto sm:w-full sm:max-w-[1280px] light">
        <div className="flex flex-row gap-6  bg-card text-foreground sm:rounded-lg h-[720px] overflow-hidden">
          <Outlet />
        </div>
      </div>
    </div>
  )
}
