import { styled } from '@stitches/react'

import type * as Stitches from '@stitches/react'
import type { FunctionComponent, SVGProps } from 'react'

interface LogoProps extends Stitches.VariantProps<typeof StyledLogo> {
  className?: string
  isDarkMode?: boolean
  size?: 'small' | 'medium' | 'large'
}

const LogoSVG: FunctionComponent<SVGProps<SVGSVGElement>> = props => (
  <svg
    width="40"
    height="40"
    viewBox="0 0 40 40"
    fill="none"
    xmlns="http://www.w3.org/2000/svg"
    {...props}
  >
    <path
      fillRule="evenodd"
      clipRule="evenodd"
      d="M33 22.2223V17.7779L28.3323 17.7779L28.3323 13.3335H23.7119V8C23.7119 8 19.6464 8.00025 18.0797 8.00025C11.5224 8.00025 7 13.9793 7 20.0001C7 26.1366 11.5224 32 18.0797 32C19.6464 32 23.7119 32 23.7119 32V26.6667L28.3323 26.6667V22.2223L33 22.2223ZM28.3323 22.2223H23.7119V17.7779L28.3323 17.7779V22.2223ZM18.9705 26.6667H23.7119V22.2223H18.9705V26.6667ZM23.7119 17.7779H18.9705V13.3335L23.7119 13.3335L23.7119 17.7779ZM14.3766 22.2223H18.9705V17.7779H14.3766L14.3766 22.2223Z"
      fill="#0E0E0F"
    />
  </svg>
)

const StyledLogo = styled(LogoSVG, {
  '&[data-dark="true"]': {
    '& path': {
      fill: '#fff',
    },
  },

  '& path': {
    transition: 'fill 100ms ease',
  },

  defaultVariants: {
    size: 'medium',
  },
  variants: {
    size: {
      small: {
        width: 24,
        height: 24,
      },

      medium: {
        width: 40,
        height: 40,
      },

      large: {
        width: 64,
        height: 64,
      },
    },
  },
})

export const LogoSymbol: FunctionComponent<LogoProps> = ({ className, isDarkMode, ...rest }) => {
  return <StyledLogo data-dark={isDarkMode} className={className} {...rest} />
}
