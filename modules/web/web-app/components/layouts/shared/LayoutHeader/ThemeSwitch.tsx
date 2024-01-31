import { ButtonAlt } from '@md/ui'
import { MoonIcon, SunIcon } from 'lucide-react'

import { useTheme } from 'providers/ThemeProvider'

export const ThemeSwitch = () => {
  const { isDarkMode, toggleTheme } = useTheme()

  return (
    <ButtonAlt
      type="default"
      onClick={toggleTheme}
      icon={
        isDarkMode ? (
          <MoonIcon size={16} strokeWidth={1.5} className="text-scale-1200" />
        ) : (
          <SunIcon size={16} strokeWidth={1.5} className="text-scale-1200" />
        )
      }
    />
  )
}
