import { createContext, useContext, useEffect, useRef, useState } from 'react'

export interface UseThemeProps {
  isInitialized: boolean
  isDarkMode: boolean
  toggleTheme: () => void
  setDarkMode: (darkMode: boolean, options?: { persistent: boolean }) => void
}

interface ThemeProviderProps {
  children?: React.ReactNode
}

export const ThemeContext = createContext<UseThemeProps>({
  isInitialized: false,
  isDarkMode: false,
  toggleTheme: () => {},
  setDarkMode: () => {},
})

export const useTheme = () => useContext(ThemeContext)

// Force the theme to `theme` while mounted, re-applying if it changes (e.g. once an
// async-loaded branding theme resolves), and restore the app's own theme on unmount.
export const useForceTheme = (theme: 'dark' | 'light') => {
  const { isDarkMode, setDarkMode, isInitialized } = useTheme()

  // Capture the app's theme once, before we first force it, so unmount restores the
  // original rather than a previously-forced value.
  const original = useRef<boolean | null>(null)

  useEffect(() => {
    if (!isInitialized) return
    if (original.current === null) original.current = isDarkMode
    setDarkMode(theme === 'dark', { persistent: false })

    return () => {
      if (original.current !== null) setDarkMode(original.current, { persistent: false })
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [isInitialized, theme])
}

const LSK = 'userpreferences_DarkMode'
export const ThemeProvider = ({ children }: ThemeProviderProps) => {
  const [isDarkMode, setIsDarkMode] = useState(false)
  const [isInitialized, setIsInitialized] = useState(false)

  useEffect(() => {
    // const key = localStorage.getItem(LSK)

    // const prefersDarkMode =
    //   window.matchMedia && window.matchMedia('(prefers-color-scheme: dark)').matches
    // // Default to dark mode if no preference config
    // const darkMode = key === 'true' || (!key && prefersDarkMode)

    // console.log('userpreferences_DarkMode', darkMode)
    setDarkMode(true)
    setIsInitialized(true)
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
          isInitialized,
        }}
      >
        {children}
      </ThemeContext.Provider>
    </>
  )
}
