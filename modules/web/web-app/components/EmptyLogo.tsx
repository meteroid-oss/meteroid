import { useTheme } from 'providers/ThemeProvider'

export function EmptyLogo({ className }: { className?: string }) {
  const { isDarkMode } = useTheme()

  return isDarkMode ? (
    <img src="/img/empty-dark.png" alt="no data" className={className} />
  ) : (
    <img src="/img/empty.png" alt="no data" className={className} />
  )
}
