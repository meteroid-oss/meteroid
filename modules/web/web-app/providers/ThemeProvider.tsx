import { createContext, useContext, useEffect, useState } from 'react'

export interface UseThemeProps {
  isDarkMode: boolean
  toggleTheme: () => void
  setDarkMode: (darkMode: boolean, options?: { persistent: boolean }) => void
}

interface ThemeProviderProps {
  children?: React.ReactNode
}

export const ThemeContext = createContext<UseThemeProps>({
  isDarkMode: false,
  toggleTheme: () => {},
  setDarkMode: () => {},
})

export const useTheme = () => useContext(ThemeContext)

// force the theme to X and restore to previous value on unmount
export const useForceTheme = (theme: 'dark' | 'light') => {
  const { isDarkMode, setDarkMode } = useTheme()

  const prevDarkMode = isDarkMode

  useEffect(() => {
    setDarkMode(theme === 'dark', { persistent: false })

    return () => {
      setDarkMode(prevDarkMode, { persistent: false })
    }
  }, [])
}

const LSK = 'userpreferences_DarkMode'
export const ThemeProvider = ({ children }: ThemeProviderProps) => {
  const [isDarkMode, setIsDarkMode] = useState(false)

  useEffect(() => {
    const key = localStorage.getItem(LSK)

    const prefersDarkMode =
      window.matchMedia && window.matchMedia('(prefers-color-scheme: dark)').matches
    // Default to dark mode if no preference config
    const darkMode = key === 'true' || (!key && prefersDarkMode)

    setDarkMode(darkMode)
  }, [])

  const setDarkMode: UseThemeProps['setDarkMode'] = (darkMode, options = { persistent: true }) => {
    options.persistent && localStorage.setItem(LSK, darkMode.toString())

    const newTheme = darkMode ? 'dark' : 'light'

    document.body.classList.remove('light', 'dark')
    document.body.classList.add(newTheme)

    // Color scheme must be applied to document element (`<html>`)
    document.documentElement.style.colorScheme = newTheme

    setIsDarkMode(darkMode)
  }

  const toggleTheme = () => {
    setDarkMode(!isDarkMode)
  }

  return (
    <>
      <ThemeContext.Provider
        value={{
          isDarkMode,
          toggleTheme,
          setDarkMode,
        }}
      >
        {children}
      </ThemeContext.Provider>
    </>
  )
}
