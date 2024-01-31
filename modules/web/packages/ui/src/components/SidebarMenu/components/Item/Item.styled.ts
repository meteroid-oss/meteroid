import { colors, fontSizes, fontWeights, radius, spaces } from '@md/foundation'
import { styled } from '@stitches/react'

export const StyledItem = styled('li', {
  a: {
    display: 'block',
    width: '100%',
  },
})

export const ItemLink = styled('span', {
  display: 'block',
  width: '100%',
  fontSize: fontSizes.fontSize2,
  fontWeight: fontWeights.medium,
  color: colors.mauve12,
  borderRadius: radius.radius3,
  lineHeight: 1,
  padding: `${spaces.space4} ${spaces.space5}`,
  transition: 'background-color 0.2s ease-in-out',

  '&:hover': {
    backgroundColor: colors.mauve3,
  },

  variants: {
    isActive: {
      true: {
        backgroundColor: colors.mauve3,
      },
    },
  },
})
