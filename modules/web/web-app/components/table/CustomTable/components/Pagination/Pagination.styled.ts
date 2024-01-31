import { colors, fontSizes, fontWeights, spaces } from '@md/foundation'
import { styled } from '@stitches/react'

export const CountInfo = styled('div', {
  fontSize: fontSizes.fontSize2,
  color: colors.neutral11,
  display: 'flex',
  alignItems: 'center',
  gap: spaces.space2,
  lineHeight: 1,

  '& > span': {
    fontWeight: fontWeights.medium,
  },
})
