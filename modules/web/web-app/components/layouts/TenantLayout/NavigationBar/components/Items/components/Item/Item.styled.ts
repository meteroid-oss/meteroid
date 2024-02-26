import { radius, spaces } from '@md/foundation'
import { keyframes, styled } from '@stitches/react'
import { NavLink } from 'react-router-dom'

export const StyledItem = styled('li', {
  width: '100%',
})

const ActiveStateKeyframe = keyframes({
  '0%': {
    transform: 'translateX(-4px)',
    opacity: 0,
  },
  '100%': {
    transform: 'translateX(0)',
    opacity: 1,
  },
})

export const ItemLink = styled(NavLink, {
  position: 'relative',
  display: 'flex',
  alignItems: 'center',
  justifyContent: 'center',
  margin: `0 ${spaces.space5}`,
  padding: `${spaces.space4} 0`,
  borderRadius: radius.radius3,
  backgroundColor: 'transparent',
  transition: 'background-color 0.2s ease-in-out',

  '&.active::before, &[data-exit="true"]::before': {
    content: '',
    position: 'absolute',
    right: `calc(${spaces.space5} * -1)`,
    top: 0,
    width: 3,
    height: '100%',
    borderTopLeftRadius: 1.5,
    borderBottomLeftRadius: 1.5,
    background: 'linear-gradient(180deg, #4F46FF -37.5%, #817AFF 100%), #7B74FF',
    animation: `${ActiveStateKeyframe} 0.3s ease-in`,
    transition: 'transform 0.3s ease-out, opacity 0.3s ease-out',
  },

  '&[data-exit="true"]::before': {
    transform: 'translateX(-4px)',
    opacity: 0,
  },
})
