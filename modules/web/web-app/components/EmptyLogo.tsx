import { useTheme } from 'providers/ThemeProvider'

export function EmptyLogo({ size }: { size: number }) {
  const { isDarkMode } = useTheme()

  return isDarkMode ? (
    <img src="/img/empty-dark.png" alt="no data" height={size} width={size} />
  ) : (
    <img src="/img/empty.png" alt="no data" height={size} width={size} />
  )
}
