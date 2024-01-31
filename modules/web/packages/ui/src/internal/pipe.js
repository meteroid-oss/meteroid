export function pipe() {
  let x = arguments[0]

  for (let i = 1, l = arguments.length; i < l; i++) {
    x = arguments[i](x)
  }

  return x
}
