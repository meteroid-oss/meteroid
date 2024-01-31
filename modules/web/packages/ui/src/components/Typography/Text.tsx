export interface TextProps {
  children: React.ReactNode
  style?: React.CSSProperties
  type?: 'default' | 'secondary' | 'success' | 'warning' | 'danger'
  mark?: boolean
  code?: boolean
  keyboard?: boolean
  strong?: boolean
}

export const Text: React.FC<TextProps> = ({ children, style, mark, code, keyboard, strong }) => {
  if (code) return <code style={style}>{children}</code>
  if (mark) return <mark style={style}>{children}</mark>
  if (keyboard) return <kbd style={style}>{children}</kbd>
  if (strong) return <strong style={style}>{children}</strong>
  return <span style={style}>{children}</span>
}
