import { colors, radius } from '@md/foundation'
import * as Switch from '@radix-ui/react-switch'
import { styled } from '@stitches/react'

export const SwitchRoot = styled(Switch.Root, {
  all: 'unset',
  width: 42,
  height: 25,
  backgroundColor: colors.neutral4,
  borderRadius: radius.pill,
  position: 'relative',
  WebkitTapHighlightColor: 'rgba(0, 0, 0, 0)',
  border: '1px solid transparent',
  transition: 'all 200ms ease-out',

  '&:focus': {
    outline: '1px solid transparent',
    boxShadow: '0px 0px 0px 3px rgba(0, 0, 0, 0.08)',
    borderColor: 'rgba(0, 0, 0, 0.22)',
  },
  '&[data-state="checked"]': { backgroundColor: colors.secondary9 },
})

export const SwitchThumb = styled(Switch.Thumb, {
  display: 'block',
  width: 21,
  height: 21,
  backgroundColor: colors.white1,
  borderRadius: radius.pill,
  transition: 'transform 100ms',
  transform: 'translateX(2px)',
  willChange: 'transform',
  '&[data-state="checked"]': { transform: 'translateX(19px)' },
})

export const Flex = styled('div', { display: 'flex' })
export const Label = styled('label', {
  color: colors.white1,
  fontSize: 15,
  lineHeight: 1,
})
