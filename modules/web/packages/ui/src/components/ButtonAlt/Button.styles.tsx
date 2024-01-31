import { default__padding_and_text, defaults } from '@ui/lib/tailwind/defaults'

export const twButtonAltStyles = {
  button: {
    base: `
          relative
          cursor-pointer
          inline-flex items-center space-x-2
          text-center
          font-regular
          transition ease-out duration-200
          rounded
          ${defaults['focus-visible']}
    
        `,
    label: `truncate`,
    container: 'inline-flex font-medium',
    type: {
      primary: `
            bg-brand-fixed-1100 hover:bg-brand-fixed-1000
            text-white-100
            dark:text-white-1200
            bordershadow-brand-fixed-1000 hover:bordershadow-brand-fixed-900 dark:bordershadow-brand-fixed-1000 dark:hover:bordershadow-brand-fixed-1000
            focus-visible:outline-brand-600
          `,
      secondary: `
            bg-scale-1200
            text-scale-100 hover:text-scale-800
            focus-visible:text-scale-600
    
            bordershadow-scale-1100 hover:bordershadow-scale-900
    
            focus-visible:outline-scale-700
          `,
      default: `
            text-scale-1200
            bg-scale-100 hover:bg-scale-300
            bordershadow-scale-600 hover:bordershadow-scale-700
            dark:bordershadow-scale-700 hover:dark:bordershadow-scale-800
            dark:bg-scale-500 dark:hover:bg-scale-600
            focus-visible:outline-brand-600
    
          `,
      alternative: `
            text-brand-1100
            bg-brand-200 hover:bg-brand-400
            bordershadow-brand-600 hover:bordershadow-brand-800
            dark:bordershadow-brand-700 hover:dark:bordershadow-brand-800
            focus-visible:border-brand-800
            focus-visible:outline-brand-600
          `,
      outline: `
            text-scale-1200
            bg-transparent
            bordershadow-scale-600 hover:bordershadow-scale-700
            dark:bordershadow-scale-800 hover:dark:bordershadow-scale-900
            focus-visible:outline-scale-700
            border
            border-scale-700 hover:border-scale-900
          `,
      dashed: `
            text-scale-1200
            border
            border-dashed
            border-scale-700 hover:border-scale-900
            bg-transparent
            focus-visible:outline-scale-700
          `,
      link: `
            text-brand-1100
            border
            border-transparent
            hover:bg-brand-400
            border-opacity-0
            bg-opacity-0 dark:bg-opacity-0
            shadow-none
            focus-visible:outline-scale-700
          `,
      text: `
            text-scale-1200
            hover:bg-scale-500
            shadow-none
            focus-visible:outline-scale-700
          `,
      danger: `
            text-red-1100
            bg-red-200
            bordershadow-red-700 hover:bordershadow-red-900
            hover:bg-red-900
            hover:text-lo-contrast
            focus-visible:outline-red-700
          `,
      warning: `
            text-amber-1100
            bg-amber-200
            bordershadow-amber-700 hover:bordershadow-amber-900
            hover:bg-amber-900
            hover:text-hi-contrast
            focus-visible:outline-amber-700
          `,
    },
    block: 'w-full flex items-center justify-center',
    shadow: 'shadow-sm',
    size: {
      ...default__padding_and_text,
    },
    loading: 'animate-spin',
    // disabled prefix is disabled by default in tailwind
    // so we apply normal utilities instead, however you can add disabled prefixes if you enabled them in tailwind config.
    // see more: https://tailwindcss.com/docs/hover-focus-and-other-states#disabled
    disabled: 'opacity-50 cursor-not-allowed pointer-events-none',
  },
}
