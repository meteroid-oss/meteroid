import { colors, fontSizes, fontWeights, spaces } from '@md/foundation'
import { styled } from '@stitches/react'

export const StyledGroup = styled('div', {
  marginTop: spaces.space6,
})

export const Items = styled('ul', {
  listStyleType: 'none',
  display: 'flex',
  flexDirection: 'column',
  gap: spaces.space2,
})

export const Label = styled('span', {
  display: 'block',
  fontSize: fontSizes.fontSize1,
  fontWeight: fontWeights.medium,
  textTransform: 'uppercase',
  color: colors.mauve9,
  marginBottom: spaces.space3,
  pointerEvents: 'none',
})
