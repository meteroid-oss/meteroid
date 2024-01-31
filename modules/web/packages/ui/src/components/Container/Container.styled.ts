import { spaces } from '@md/foundation'
import { styled } from '@stitches/react'

export const StyledContainer = styled('div', {
  width: '100%',
  display: 'block',
  position: 'relative',
  paddingInline: spaces.space10,
  paddingBlock: spaces.space8,

  variants: {
    fullHeight: {
      true: {
        height: '100%',
      },
    },
  },
})
