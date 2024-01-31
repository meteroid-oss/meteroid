interface Props {
  children?: React.ReactNode
  style?: React.CSSProperties
}

export default function Divider({ children, style }: Props) {
  return (
    <div role="separator" style={style}>
      {children && <span>{children}</span>}
    </div>
  )
}
