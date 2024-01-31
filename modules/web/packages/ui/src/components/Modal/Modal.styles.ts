export const twModalStyles = {
  modal: {
    base: `
          relative
          bg-scale-100 dark:bg-scale-300
          my-4
          border border-scale-500 dark:border-scale-500
          rounded-xl
          shadow-xl
          data-open:animate-overlay-show
          data-closed:animate-overlay-hide
        `,
    header: `
          bg-scale-200 dark:bg-scale-400
          space-y-1 py-3 px-4 sm:px-5
          border-b border-scale-300 dark:border-scale-500
          rounded-xl rounded-b-none
        `,
    footer: `
          flex justify-end gap-2
          py-3 px-5
          border-t border-scale-300 dark:border-scale-500
        `,
    size: {
      tiny: `sm:align-middle sm:w-full sm:max-w-xs`,
      small: `sm:align-middle sm:w-full sm:max-w-sm`,
      medium: `sm:align-middle sm:w-full sm:max-w-lg`,
      large: `sm:align-middle sm:w-full max-w-xl`,
      xlarge: `sm:align-middle sm:w-full max-w-3xl`,
      xxlarge: `sm:align-middle sm:w-full max-w-6xl`,
      xxxlarge: `sm:align-middle sm:w-full max-w-7xl`,
    },
    overlay: `
          z-40
          fixed
          bg-scale-300
          dark:bg-scale-100
          h-full w-full
          left-0
          top-0
          opacity-75
          data-closed:animate-fade-out-overlay-bg
          data-open:animate-fade-in-overlay-bg
        `,
    scroll_overlay: `
          z-40
          fixed
          inset-0
          grid
          place-items-center
          overflow-y-auto
          data-open:animate-overlay-show data-closed:animate-overlay-hide
        `,
    separator: `
          w-full
          h-px
          my-2
          bg-scale-300 dark:bg-scale-500
        `,
    content: `px-5`,
  },
}
