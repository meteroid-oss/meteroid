interface SpaceProps {
  className?: string
  style?: React.CSSProperties
  children?: React.ReactNode
}

export const Space = ({ className, style, children }: SpaceProps) => {
  return (
    <div className={className} style={style}>
      {children}
    </div>
  )
}
