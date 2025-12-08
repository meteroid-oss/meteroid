import { cn } from '@ui/lib'

import type { FunctionComponent, SVGProps } from 'react'

interface LogoProps {
  className?: string
  isDarkMode?: boolean
  size?: 'small' | 'medium' | 'large'
}

const sizeClasses = {
  small: 'w-6 h-6',
  medium: 'w-10 h-10',
  large: 'w-16 h-16',
}

const LogoSVG: FunctionComponent<SVGProps<SVGSVGElement>> = props => (
  <svg viewBox="0 0 120 120" fill="none" xmlns="http://www.w3.org/2000/svg" {...props}>
    <path
      fillRule="evenodd"
      clipRule="evenodd"
      d="M2.17987 10.9202C0 15.1984 0 20.799 0 32V88C0 99.2011 0 104.802 2.17987 109.08C4.09734 112.843 7.15695 115.903 10.9202 117.82C15.1984 120 20.799 120 32 120H88C99.2011 120 104.802 120 109.08 117.82C112.843 115.903 115.903 112.843 117.82 109.08C120 104.802 120 99.201 120 88V32C120 20.7989 120 15.1984 117.82 10.9202C115.903 7.15695 112.843 4.09734 109.08 2.17987C104.802 0 99.201 0 88 0H32C20.7989 0 15.1984 0 10.9202 2.17987C7.15695 4.09734 4.09734 7.15695 2.17987 10.9202ZM53.3333 66.6667H20V100H53.3333V66.6667Z"
      fill="currentColor"
    />
  </svg>
)

export const LogoSymbol: FunctionComponent<LogoProps> = ({
  className,
  isDarkMode,
  size = 'small',
}) => {
  return (
    <LogoSVG
      className={cn(
        sizeClasses[size],
        isDarkMode ? 'text-white' : 'text-[#030008]',
        'transition-colors duration-100',
        className
      )}
    />
  )
}
