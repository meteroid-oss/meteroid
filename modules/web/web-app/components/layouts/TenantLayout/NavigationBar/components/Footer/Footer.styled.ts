import { colors, radius, spaces } from '@md/foundation'
import { styled } from '@stitches/react'

export const StyledFooter = styled('footer', {
  width: '100%',
})

export const Avatar = styled('img', {
  borderRadius: radius.round,
})

export const AvatarTrigger = styled('li', {
  width: `calc(100% - ${spaces.space5} * 2)`,
  display: 'flex',
  alignItems: 'center',
  justifyContent: 'center',
  position: 'relative',
  margin: `0 ${spaces.space5}`,
  padding: `${spaces.space4} 0`,
  borderRadius: radius.radius3,
  backgroundColor: 'transparent',
  transition: 'background-color 0.2s ease-in-out',

  '&:hover, &.active': {
    backgroundColor: colors.mauve4,
  },
})
