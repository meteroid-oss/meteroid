import { colors, fontFace, fontFamilies, fontWeights, fonts } from '@md/foundation'
import { globalCss } from '@stitches/react'

export const globalStyles = globalCss({
  // eslint-disable-next-line @typescript-eslint/ban-ts-comment
  // @ts-ignore
  '@font-face': fontFamilies.map(font =>
    fontFace(font.familyName, font.fontPath, font.fontWeight, font.fontStyle)
  ),

  '*': {
    fontFamily: fonts.sans,
  },

  'html, body': {
    fontWeight: fontWeights.regular,
    fontSize: 16,
    lineHeight: '1.75 !important',
    color: colors.primary11,
  },
})
