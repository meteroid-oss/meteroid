import { Flex } from './Flex'

import type { Meta, StoryObj } from '@storybook/react'

const meta: Meta<typeof Flex> = {
  title: 'Flex',
  component: Flex,
  tags: ['autodocs'],
  argTypes: {
    direction: {
      control: {
        type: 'select',
        options: ['row', 'column'],
      },
    },
    justify: {
      control: {
        type: 'select',
        options: [
          'flex-start',
          'flex-end',
          'center',
          'space-between',
          'space-around',
          'space-evenly',
        ],
      },
    },
    align: {
      control: {
        type: 'select',
        options: ['flex-start', 'flex-end', 'center', 'baseline', 'stretch'],
      },
    },
  },
}

export default meta
type Story = StoryObj<typeof Flex>

export const Default: Story = {
  args: {
    direction: 'row',
    justify: 'flex-start',
    align: 'flex-start',
  },
}
