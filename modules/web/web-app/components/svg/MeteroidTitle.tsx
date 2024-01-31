import SVG from 'react-inlinesvg'

import { useTheme } from 'providers/ThemeProvider'

interface Props {
  className?: string
}
export const MeteroidTitle = ({ className }: Props) => {
  const { isDarkMode } = useTheme()

  const fill = isDarkMode ? '#fff' : '#0B0B0B'
  const defaultClassName = 'h-5 p-0'

  return <SVG src="/img/meteroid-title.svg" fill={fill} className={className ?? defaultClassName} />
}
