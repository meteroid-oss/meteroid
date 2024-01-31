export const fontFace = (
  fontFamily: string,
  fontPath: string,
  fontWeight: number | string,
  fontStyle = 'normal'
) => ({
  fontFamily,
  fontDisplay: 'swap',
  fontStyle,
  fontWeight,
  src: `url('/fonts/${fontPath}.woff2?v=3.19') format('woff2'), url('/fonts/${fontPath}.woff?v=3.19') format('woff')`,
})
