import { createContext, useContext, useEffect, useState } from 'react'

export interface UseThemeProps {
  isDarkMode: boolean
  toggleTheme: () => void
  setDarkMode: (darkMode: boolean) => void
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

const LSK = 'userpreferences_DarkMode'
export const ThemeProvider = ({ children }: ThemeProviderProps) => {
  const [isDarkMode, setIsDarkMode] = useState(false)

  useEffect(() => {
    const key = localStorage.getItem(LSK)

    // Default to dark mode if no preference config
    const mode = !key || key === 'true'

    setDarkMode(mode)
  }, [])

  const setDarkMode: UseThemeProps['setDarkMode'] = darkMode => {
    localStorage.setItem(LSK, darkMode.toString())

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
