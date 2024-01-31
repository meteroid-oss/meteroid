export const twPopoverStyles = {
  popover: {
    trigger: `
          flex
          border-none
    
          rounded
          bg-transparent
          p-0
          outline-none
          outline-offset-1
          transition-all
          focus:outline-4
          focus:outline-scale-600
    
        `,
    content: `
          z-40
          bg-scale-100 dark:bg-scale-300
          border border-scale-300 dark:border-scale-500
          rounded
          shadow-lg
          data-open:animate-dropdown-content-show
          data-closed:animate-dropdown-content-hide
          min-w-fit
    
          origin-popover
          data-open:animate-dropdown-content-show
          data-closed:animate-dropdown-content-hide
        `,
    size: {
      tiny: `w-40`,
      small: `w-48`,
      medium: `w-64`,
      large: `w-80`,
      xlarge: `w-96`,
      content: `w-auto`,
    },
    header: `
          bg-scale-200 dark:bg-scale-400
          space-y-1 py-1.5 px-3
          border-b border-scale-300 dark:border-scale-500
        `,
    footer: `
          bg-scale-200 dark:bg-scale-400
          py-1.5 px-3
          border-t border-scale-300 dark:border-scale-500
        `,
    close: `
          transition
          text-scale-900 hover:text-scale-1100
        `,
    separator: `
          w-full
          h-px
          my-2
          bg-scale-300 dark:bg-scale-500
        `,
  },
}
