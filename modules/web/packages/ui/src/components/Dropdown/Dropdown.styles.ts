export const twDropdownStyles = {
  dropdown: {
    trigger: `
          flex
    
          border-none
          rounded
          bg-transparent p-0
          outline-none
          outline-offset-1
          transition-all
          focus:outline-4
          focus:outline-scale-600
        `,
    item_nested: `
          border-none
          focus:outline-none
          focus:bg-scale-300 dark:focus:bg-scale-500
          focus:text-scale-1200
          data-open:bg-scale-300 dark:data-open:bg-scale-500
          data-open:text-scale-1200
        `,
    content: `
          z-40
          bg-scale-100 dark:bg-scale-300
          border border-scale-300 dark:border-scale-500
          rounded
          shadow-lg
          py-1.5
          origin-dropdown
          data-open:animate-dropdown-content-show
          data-closed:animate-dropdown-content-hide
          min-w-fit
        `,
    size: {
      tiny: `w-40`,
      small: `w-48`,
      medium: `w-64`,
      large: `w-80`,
      xlarge: `w-96`,
      content: `w-auto`,
    },
    arrow: `
          fill-current
          border-0 border-t
        `,
    item: `
          group
          relative
          text-xs
          text-scale-1100
          px-4 py-1.5 flex items-center space-x-2
          cursor-pointer
          focus:bg-scale-300 dark:focus:bg-scale-500
          focus:text-scale-1200
          border-none
          focus:outline-none
        `,
    disabled: `opacity-50 cursor-default`,
    label: `
          text-scale-900
          px-4 flex items-center space-x-2 py-1.5
          text-xs
        `,
    separator: `
          w-full
          h-px
          my-2
          bg-scale-300 dark:bg-scale-500
        `,
    misc: `
          px-4 py-1.5
        `,
    check: `
          absolute left-3
          flex items-center
          data-checked:text-scale-1200
        `,
    input: `
          flex items-center space-x-0 pl-8 pr-4
        `,
    right_slot: `
          text-scale-900
          group-focus:text-scale-1000
          absolute
          -translate-y-1/2
          right-2
          top-1/2
        `,
  },
}
