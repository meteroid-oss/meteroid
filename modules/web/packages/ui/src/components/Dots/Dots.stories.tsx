import { Story } from '@storybook/react'

import { Dots, DotsProps } from './Dots'

export default {
  title: 'Components/Dots',
  component: Dots,
}

const Template: Story<DotsProps> = args => <Dots {...args} />

export const Default = Template.bind({})
Default.args = {}

export const LargeDots = Template.bind({})
LargeDots.args = {
  size: 'large',
}

export const DarkDots = Template.bind({})
DarkDots.args = {
  variant: 'dark',
}
