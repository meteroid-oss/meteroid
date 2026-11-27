import { memo } from 'react'
import SVG from 'react-inlinesvg'

import { useTheme } from 'providers/ThemeProvider'

interface Props {
  width?: number
  height?: number
  forceTheme?: 'dark' | 'light'
}

// The mark and the wordmark side by side, as designed: 133 × 31.65, about 4.2 : 1.
export const MeteroidTitle = memo(({ width = 101, height = 24, forceTheme }: Props) => {
  const { isDarkMode } = useTheme()

  const enforceDarkMode = forceTheme === 'dark' || (forceTheme === undefined && isDarkMode)

  return (
    <div className="flex h-7 items-center">
      <SVG
        src={`/img/meteroid-logo-wordmark--${enforceDarkMode ? 'dark' : 'light'}.svg`}
        width={width}
        height={height}
      />
    </div>
  )
})
