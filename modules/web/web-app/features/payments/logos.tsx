import { FunctionComponent } from 'react'
import { siStripe } from 'simple-icons'

export const BrandIcon = ({
  path,
  color,
  className,
}: {
  path: string
  color: string
  className?: string
}) => (
  <svg viewBox="0 0 24 24" fill={color} className={className}>
    <path d={path} />
  </svg>
)

/** GoCardless mark (the site favicon): dark "G" on the brand yellow disc. */
export const GoCardlessLogo = ({ className }: { className?: string }) => (
  <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64" fill="none" className={className}>
    <path
      fill="#F1F252"
      d="M32 64c17.673 0 32-14.327 32-32S49.673 0 32 0 0 14.327 0 32s14.327 32 32 32"
    />
    <path
      fill="#1C1B18"
      d="M32.505 15.5c3.517 0 5.497.578 5.497.578l5.846 11.937-.052.052-7.526-4.487c-4.36-2.597-7.528-3.962-10.098-3.858-2.724.052-4.358 2.242-4.358 5.421.099 8.134 7.826 18.141 15.502 18.141 3.133 0 4.764-1.007 5.718-2.227L31.668 28.585v-.051h15.61c.214 1.117.33 2.25.347 3.388 0 9.123-6.983 16.473-15.599 16.473s-15.651-7.35-15.651-16.473C16.36 22.849 23.344 15.5 32.505 15.5"
    />
  </svg>
)

/** Mollie's "m" letterform (the app icon), cropped from the official wordmark. */
export const MollieLogo = ({ className }: { className?: string }) => (
  <svg xmlns="http://www.w3.org/2000/svg" viewBox="-60 80 810 810" className={className}>
    <path
      fill="currentColor"
      d="m549.6 241.4c-6.5-.5-12.7-.8-19.1-.8-60 0-116.9 24.6-157.7 68-40.8-43.2-97.5-68-156.9-68-119 .1-215.9 96.7-215.9 215.7v272.2h116.3v-268.9c0-49.4 40.6-94.9 88.4-99.8 3.4-.3 6.7-.5 9.8-.5 53.8 0 97.7 43.9 98 97.7v271.4h118.9v-269.3c0-49.1 40.3-94.6 88.4-99.5 3.4-.3 6.7-.5 9.8-.5 53.8 0 98 43.7 98.2 97.2v272.2h118.9v-268.9c0-54.5-20.2-107.1-56.6-147.6-36.3-40.8-86.2-65.9-140.5-70.6z"
    />
  </svg>
)

// Stancer's logo is a gradient, so it's inlined instead of using BrandIcon; ids are prefixed
// to avoid DOM collisions.
export const StancerLogo = ({ className }: { className?: string }) => (
  <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 104 104" className={className}>
    <defs>
      <linearGradient
        id="stancer-logo-a"
        x1="57.242"
        y1="35"
        x2="1.041"
        y2="80.666"
        gradientUnits="userSpaceOnUse"
      >
        <stop stopColor="#215DD2" />
        <stop offset="1" stopColor="#79A7FF" />
      </linearGradient>
      <linearGradient
        id="stancer-logo-b"
        x1="82.862"
        y1="7.249"
        x2="41.529"
        y2="53.322"
        gradientUnits="userSpaceOnUse"
      >
        <stop stopColor="#d0e9ff" />
        <stop offset="1" stopColor="#FF5B58" />
      </linearGradient>
      <filter
        id="stancer-logo-c"
        x="24.534"
        y="29.266"
        width="58.749"
        height="44.733"
        filterUnits="userSpaceOnUse"
        colorInterpolationFilters="sRGB"
      >
        <feFlood floodOpacity="0" result="BackgroundImageFix" />
        <feBlend in="SourceGraphic" in2="BackgroundImageFix" result="shape" />
        <feGaussianBlur stdDeviation="2" result="effect1_foregroundBlur" />
      </filter>
    </defs>
    <path
      d="M35.656 32h-11.57C17.239 32 11.28 36.6 9.664 43.131L.667 83.509C-.305 87.873 3.092 92 7.657 92h51.27c4.528 0 8.446-3.09 9.422-7.43l9.973-44.347C79.272 36 76.313 32 71.908 32H35.656Z"
      fill="url(#stancer-logo-a)"
    />
    <path
      d="M51.509 12h46.18c4.352 0 7.587 4 6.649 8.223l-.965 4.343H36.908l.35-1.435C38.856 16.599 44.744 12 51.509 12ZM35.112 31.913l-8.486 34.72C25.958 69.367 28.042 72 30.873 72h54.303c4.473 0 8.344-3.09 9.309-7.43l7.256-32.657H35.112Z"
      fill="#d0e9ff"
      fillRule="evenodd"
    />
    <path
      d="m71.563 70 7.72-36.734H36.695L28.66 64.633c-.668 2.733 1.416 5.366 4.247 5.366h38.656Z"
      fill="url(#stancer-logo-b)"
      fillRule="evenodd"
      filter="url(#stancer-logo-c)"
    />
  </svg>
)

export const StripeLogo = ({ className }: { className?: string }) => (
  <BrandIcon path={siStripe.path} color="#635bff" className={className} />
)

export type ProviderLogo = FunctionComponent<{ className?: string }>
