export interface LinkProps {
  children?: React.ReactNode
  target?: '_blank' | '_self' | '_parent' | '_top' | 'framename'
  href?: string
  className?: string
  style?: React.CSSProperties
  onClick?: React.MouseEventHandler<HTMLAnchorElement>
}
export const Link: React.FC<LinkProps> = ({
  children,
  target = '_blank',
  href,
  onClick,
  style,
}) => {
  return (
    <a onClick={onClick} href={href} target={target} rel="noopener noreferrer" style={style}>
      {children}
    </a>
  )
}
