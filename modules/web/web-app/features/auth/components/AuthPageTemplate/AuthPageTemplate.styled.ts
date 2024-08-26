import { colors, fontWeights, radius, spaces } from '@md/foundation'
import { styled } from '@stitches/react'

export const StyledPageTemplate = styled('div', {
  height: '100%',
  display: 'grid',
  alignItems: 'center',
  justifyContent: 'center',
})

export const ContentWrapper = styled('div', {
  display: 'grid',
  gridTemplateColumns: '500px 1fr',
  backgroundColor: colors.neutral1,
  boxShadow: '0px 4px 22px -14px rgba(0, 0, 0, 0.15), 0px 54px 72px -40px rgba(0, 0, 0, 0.15)',
  width: 896,
  height: 576,
  margin: 'auto auto',
  borderRadius: radius.radius5,
})

export const FormContainer = styled('div', {
  padding: spaces.space12,
})

export const FormContainerHeader = styled('header', {
  display: 'flex',
  flexDirection: 'column',
  gap: spaces.space9,
})

export const Tabs = styled('ul', {
  listStyleType: 'none',
  display: 'grid',
  gridTemplateColumns: '1fr 1fr',
  marginBottom: spaces.space8,
  borderBottom: `1px solid ${colors.mauve5}`,
})

export const Tab = styled('li', {
  display: 'flex',
  justifyContent: 'center',
  alignItems: 'center',

  a: {
    fontWeight: fontWeights.medium,
    letterSpacing: -0.2,
    textAlign: 'center',
    color: colors.mauve11,
    paddingBottom: spaces.space6,
    borderBottom: '2px solid transparent',
    width: '100%',
    transition: 'color 0.2s ease-in-out, border-bottom 0.2s ease-in-out',

    '&:hover': {
      color: colors.mauve12,
    },
  },

  variants: {
    isActive: {
      true: {
        a: {
          borderBottom: `2px solid ${colors.mauve12}`,
          color: colors.mauve12,
        },
      },
    },
  },
})

export const Visual = styled('div', {
  backgroundImage: 'url(/img/auth.png)',
  backgroundSize: 'cover',
  backgroundPosition: 'center',
  backgroundRepeat: 'no-repeat',
  borderTopRightRadius: radius.radius5,
  borderBottomRightRadius: radius.radius5,
})
