import { Link } from './Link'
import { Text } from './Text'
import { Title } from './Title'

interface Props {
  children?: React.ReactNode
  style?: React.CSSProperties
  tag?: string
}
function Typography({ children, tag = 'div', style }: Props) {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const CustomTag: any = `${tag}`
  return <CustomTag style={style}>{children}</CustomTag>
}

Typography.Title = Title
Typography.Text = Text
Typography.Link = Link

export default Typography
