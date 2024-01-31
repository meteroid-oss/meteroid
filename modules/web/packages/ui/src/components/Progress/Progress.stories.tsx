import { Progress } from './Progress'

import type { Meta, StoryObj } from '@storybook/react'

const meta: Meta<typeof Progress> = {
  title: 'Progress',
  component: Progress,
  tags: ['autodocs'],
  argTypes: {
    value: {
      control: {
        type: 'range',
        min: 0,
        max: 100,
        step: 1,
      },
    },
  },
}

export default meta
type Story = StoryObj<typeof Progress>

export const Default: Story = {
  args: {
    value: 33,
  },
}
