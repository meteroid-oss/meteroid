import { Flex, Switch } from '@ui/components'

import { StyledNewsletterSubscription } from './NewsletterSubscription.styled'

import type { FunctionComponent } from 'react'

const NewsletterSubscription: FunctionComponent = () => {
  return (
    <Flex direction="row" justify="space-between" align="center">
      <StyledNewsletterSubscription>
        <span>Subscribe to product update emails</span>
        <span>Get the latest updates about product updates</span>
      </StyledNewsletterSubscription>
      <Switch id="newsletter-subscription" />
    </Flex>
  )
}

export default NewsletterSubscription
