import { colors, fontSizes, radius, spaces } from '@md/foundation'
import * as SelectPrimitive from '@radix-ui/react-select'
import { styled } from '@stitches/react'

export const StyledTrigger = styled(SelectPrimitive.Trigger, {
  backgroundColor: colors.neutral1,
  color: colors.neutral12,
  fontSize: fontSizes.fontSize2,
  padding: `${spaces.space4} ${spaces.space6}`,
  borderRadius: radius.radius3,
  border: `1px solid hsla(0, 0%, 0%, 0.12)`,
  display: 'flex',
  alignItems: 'center',
  justifyContent: 'space-between',
  lineHeight: 1,
  gap: spaces.space2,
  transition: 'box-shadow 200ms ease-out, color 200ms ease-out, border-color 200ms ease-out',

  '& span': {
    color: colors.neutral12,
  },

  '&:focus, &[data-state="open"]': {
    outline: '1px solid transparent',
    boxShadow: `0px 0px 0px 2px #F3F2F1, inset 0px 0px 1px 1.5px rgba(196, 196, 196, 0.5)`,
    borderColor: 'rgba(0, 0, 0, 0.22)',
  },

  defaultVariants: {
    size: 'medium',
  },
  variants: {
    size: {
      tiny: {
        height: 28,
        padding: `${spaces.space2} 10px`,
      },
      small: {
        height: 32,
        padding: `${spaces.space3} ${spaces.space5}`,
      },
      medium: {
        height: 36,
        padding: `${spaces.space4} ${spaces.space6}`,
      },
      large: {
        height: 40,
        padding: `10px ${spaces.space6}`,
      },
    },
  },
})

export const StyledContent = styled(SelectPrimitive.Content, {
  backgroundColor: colors.neutral1,
  boxShadow:
    '0px 8px 8px rgba(0, 0, 0, 0.05), 0px 2px 2px rgba(0, 0, 0, 0.05), 0px 1px 1px rgba(0, 0, 0, 0.05), inset 0px 1px 0px rgba(209, 209, 209, 0.25)',
  borderRadius: radius.radius3,
  border: `1px solid ${colors.neutral3}`,
  animation: 'fadeIn 200ms ease',
  padding: spaces.space2,
  width: '100% !important',
  boxSizing: 'border-box',
})

export const StyledItem = styled(SelectPrimitive.Item, {
  padding: `${spaces.space2} ${spaces.space4}`,
  borderRadius: radius.radius3,
  fontSize: fontSizes.fontSize2,
  transition: 'background-color 200ms ease-out',

  '&:not(:last-child)': {
    marginBottom: spaces.space1,
  },

  '&:hover': {
    backgroundColor: colors.neutral2,
  },
})

export const StyledItemIndicator = styled(SelectPrimitive.ItemIndicator, {
  position: 'absolute',
  right: spaces.space4,
})

export const StyledViewport = styled(SelectPrimitive.Viewport, {
  minWidth: '208px !important',
})
