import { createContext, useContext, useEffect, useRef, useState } from 'react'

export interface UseThemeProps {
  isInitialized: boolean
  isDarkMode: boolean
  toggleTheme: () => void
  setDarkMode: (darkMode: boolean, options?: { persistent: boolean }) => void
  /** An experimental dark theme layered over dark mode, or `default` for none. */
  darkVariant: DarkVariant
  setDarkVariant: (variant: DarkVariant) => void
}

interface ThemeProviderProps {
  children?: React.ReactNode
}

export const ThemeContext = createContext<UseThemeProps>({
  isInitialized: false,
  isDarkMode: false,
  toggleTheme: () => {},
  setDarkMode: () => {},
  darkVariant: 'default',
  setDarkVariant: () => {},
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
// Experimental dark themes: a class on <body> that refines `.dark` (see main.css), shown only
// while dark mode is on. Horizon leans warm, Moss green.
export const DARK_VARIANTS = ['default', 'horizon', 'moss'] as const
export type DarkVariant = (typeof DARK_VARIANTS)[number]
export const DARK_VARIANT_LABELS: Record<DarkVariant, string> = {
  default: 'Default',
  horizon: 'Horizon',
  moss: 'Moss',
}
const VARIANT_LSK = 'userpreferences_DarkVariant'
const readVariant = (): DarkVariant => {
  const stored = localStorage.getItem(VARIANT_LSK)
  return (DARK_VARIANTS as readonly string[]).includes(stored ?? '')
    ? (stored as DarkVariant)
    : 'default'
}

export const ThemeProvider = ({ children }: ThemeProviderProps) => {
  const [isDarkMode, setIsDarkMode] = useState(false)
  const [isInitialized, setIsInitialized] = useState(false)
  const [darkVariant, setVariantState] = useState<DarkVariant>(readVariant)

  useEffect(() => {
    for (const variant of DARK_VARIANTS) {
      document.body.classList.toggle(
        `theme-${variant}`,
        variant !== 'default' && variant === darkVariant
      )
    }
  }, [darkVariant])

  const setDarkVariant = (variant: DarkVariant) => {
    localStorage.setItem(VARIANT_LSK, variant)
    setVariantState(variant)
  }

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
          darkVariant,
          setDarkVariant,
        }}
      >
        {children}
      </ThemeContext.Provider>
    </>
  )
}
