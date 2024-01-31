export const twMenuClasses = {
  menu: {
    item: {
      base: `
            cursor-pointer
            flex space-x-3 items-center
            outline-none
            focus-visible:ring-1 ring-scale-1200 focus-visible:z-10
            group
          `,
      content: {
        base: `transition truncate text-sm w-full`,
        normal: `text-scale-1100 group-hover:text-scale-1200`,
        active: `text-scale-1200 font-semibold`,
      },
      icon: {
        base: `transition truncate text-sm`,
        normal: `text-scale-900 group-hover:text-scale-1100`,
        active: `text-scale-1200`,
      },
      variants: {
        text: {
          base: `
                py-1
              `,
          normal: `
                font-normal
                border-scale-500
                group-hover:border-scale-900`,
          active: `
                font-semibold
                text-scale-900
                z-10
              `,
        },
        border: {
          base: `
                px-4 py-1
              `,
          normal: `
                border-l
                font-normal
                border-scale-500
                group-hover:border-scale-900`,
          active: `
                font-semibold
    
                text-scale-900
                z-10
    
                border-l
                border-brand-900
                group-hover:border-brand-900
              `,
          rounded: `rounded-md`,
        },
        pills: {
          base: `
                px-3 py-1
              `,
          normal: `
                font-normal
                border-scale-500
                group-hover:border-scale-900`,
          active: `
                font-semibold
                bg-scale-400
                dark:bg-scale-300
                text-scale-900
                z-10
    
                rounded-md
              `,
        },
      },
    },
    group: {
      base: `
            flex space-x-3
            mb-2
            font-normal
          `,
      icon: `text-scale-900`,
      content: `text-sm text-scale-900 w-full`,
      variants: {
        text: ``,
        pills: `px-3`,
        border: ``,
      },
    },
  },
}
