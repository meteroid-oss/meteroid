export interface TitleProps {
  className?: string
  level?: 1 | 2 | 3 | 4 | 5
  children: React.ReactNode
  style?: React.CSSProperties
}

export const Title: React.FC<TitleProps> = ({ level = 1, children, style }) => {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const CustomTag: any = `h${level}`

  return <CustomTag style={style}>{children}</CustomTag>
}

export default Title
