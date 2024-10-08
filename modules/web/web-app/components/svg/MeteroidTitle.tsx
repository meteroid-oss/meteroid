import { memo } from 'react'
import SVG from 'react-inlinesvg'

import { useTheme } from 'providers/ThemeProvider'

interface Props {
  width?: number
  height?: number
  forceTheme?: 'dark' | 'light'
}

export const MeteroidTitle = memo(({ width = 120, height = 25, forceTheme }: Props) => {
  const { isDarkMode } = useTheme()

  const enforceDarkMode = forceTheme === 'dark' || (forceTheme === undefined && isDarkMode)

  return (
    <div className="w-32 h-7">
      <SVG
        src={`/img/meteroid-logo-wordmark--${enforceDarkMode ? 'dark' : 'light'}.svg`}
        width={width}
        height={height}
      />
    </div>
  )
})
