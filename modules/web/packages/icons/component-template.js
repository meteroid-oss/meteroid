const template = (variables, { tpl }) => {
  return tpl`
interface Props {
  size?: number
  fill?: string
  stroke?: string
}

export const ${variables.componentName.replace('Svg', '') + 'Icon'} = (props: Props) => {
  return (
    ${renderComponent(variables.jsx)}
  )
}
`
}

const renderComponent = _jsx => {
  let jsx = _jsx

  jsx.openingElement.attributes = _jsx.openingElement.attributes.map(attr => {
    if (attr.name.name === 'width') attr.value.expression.value = 'props.size'
    if (attr.name.name === 'height') attr.value.expression.value = 'props.size'

    return attr
  })

  jsx.openingElement.attributes.push({
    type: 'JSXAttribute',
    name: { type: 'JSXIdentifier', name: 'viewBox' },
    value: { type: 'StringLiteral', value: '0 0 16 16' },
  })

  return jsx
}

module.exports = template
