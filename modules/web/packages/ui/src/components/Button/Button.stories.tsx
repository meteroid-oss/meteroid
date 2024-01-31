import { colors } from '@md/foundation'
import { CardIcon, HomeIcon, PlusIcon } from '@md/icons'
import { Meta, Story } from '@storybook/react'

import { Button, ButtonProps } from './Button'

export default {
  title: 'Components/Button',
  component: Button,
  argTypes: {
    type: {
      control: {
        type: 'select',
        options: ['primary', 'secondary', 'tertiary', 'muted', 'success', 'warning', 'danger'],
      },
    },
    rounded: {
      control: {
        type: 'boolean',
      },
    },
    disabled: {
      control: {
        type: 'boolean',
      },
    },
    loading: {
      control: {
        type: 'boolean',
      },
    },
    transparent: {
      control: {
        type: 'boolean',
      },
    },
    size: {
      control: {
        type: 'select',
        options: ['tiny', 'small', 'medium', 'large'],
      },
    },
  },
} as Meta

const Template: Story<ButtonProps> = args => <Button {...args}>Button</Button>

export const Primary = Template.bind({})
Primary.args = {
  variant: 'primary',
}

export const Secondary = Template.bind({})
Secondary.args = {
  variant: 'secondary',
}

export const Tertiary = Template.bind({})
Tertiary.args = {
  variant: 'tertiary',
}

export const Muted = Template.bind({})
Muted.args = {
  variant: 'muted',
}

export const Success = Template.bind({})
Success.args = {
  variant: 'success',
}

export const Warning = Template.bind({})
Warning.args = {
  variant: 'warning',
}

export const Danger = Template.bind({})
Danger.args = {
  variant: 'danger',
}

export const Rounded = Template.bind({})
Rounded.args = {
  rounded: true,
}

export const Disabled = Template.bind({})
Disabled.args = {
  disabled: true,
}

export const Loading = Template.bind({})
Loading.args = {
  loading: true,
}

export const Transparent = Template.bind({})
Transparent.args = {
  transparent: true,
}

export const TinySize = Template.bind({})
TinySize.args = {
  size: 'tiny',
}

export const SmallSize = Template.bind({})
SmallSize.args = {
  size: 'small',
}

export const MediumSize = Template.bind({})
MediumSize.args = {
  size: 'medium',
}

export const LargeSize = Template.bind({})
LargeSize.args = {
  size: 'large',
}

export const RightIcon: Story<ButtonProps> = args => (
  <Button {...args}>
    <CardIcon size={16} fill={colors.neutral1} />
    Button
  </Button>
)
RightIcon.args = {
  variant: 'primary',
}

export const LeftIcon: Story<ButtonProps> = args => (
  <Button {...args}>
    Button
    <PlusIcon size={16} fill={colors.neutral1} />
  </Button>
)
LeftIcon.args = {
  variant: 'secondary',
}

export const IconOnly: Story<ButtonProps> = args => (
  <Button {...args}>
    <HomeIcon size={16} stroke={colors.neutral1} />
  </Button>
)
IconOnly.args = {
  variant: 'danger',
}
