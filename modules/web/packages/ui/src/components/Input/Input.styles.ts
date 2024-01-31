import { default__padding_and_text, defaults } from '@ui/lib/tailwind/defaults'

export const twInputAltStyles = {
  input: {
    base: `
          block
          box-border
          w-full
          rounded-md
          shadow-sm
          transition-all
          text-scale-1200
          border
          focus:shadow-md
          ${defaults.focus}
          focus:border-scale-900
          focus:ring-scale-400
          ${defaults.placeholder}
        `,
    variants: {
      standard: `
            bg-scaleA-200
            border border-scale-700
            `,
      error: `
            border border-red-800
            focus:ring-red-800
            placeholder:text-red-600
           `,
    },
    container: 'relative',
    with_icon: '!pl-10',
    size: {
      ...default__padding_and_text,
    },
    disabled: 'opacity-50',
    actions_container: 'absolute inset-y-0 right-0 pl-3 pr-1 flex space-x-1 items-center',
    textarea_actions_container: 'absolute inset-y-1.5 right-0 pl-3 pr-1 flex space-x-1 items-start',
    textarea_actions_container_items: 'flex items-center',
  },
}
