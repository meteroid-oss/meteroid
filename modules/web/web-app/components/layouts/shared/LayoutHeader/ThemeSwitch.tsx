import { Button } from '@ui2/components'
import { MoonIcon, SunIcon } from 'lucide-react'

import { useTheme } from 'providers/ThemeProvider'

export const ThemeSwitch = () => {
  const { isDarkMode, toggleTheme } = useTheme()

  return (
    <Button onClick={toggleTheme} variant="ghost" size="icon">
      {isDarkMode ? (
        <MoonIcon size={16} strokeWidth={1.5} className="text-slate-1200" />
      ) : (
        <SunIcon size={16} strokeWidth={1.5} className="text-slate-1200" />
      )}
    </Button>
  )
}
