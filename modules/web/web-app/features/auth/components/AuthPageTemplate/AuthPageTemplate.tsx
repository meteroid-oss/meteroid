import { Link, Outlet, useLocation } from 'react-router-dom'

import { MeteroidTitle } from '@/components/svg'
import {
  ContentWrapper,
  FormContainer,
  FormContainerHeader,
  StyledPageTemplate,
  Tab,
  Tabs,
  Visual,
} from '@/features/auth/components/AuthPageTemplate/AuthPageTemplate.styled'

import type { FC } from 'react'

interface PageTemplateProps {}

const PageTemplate: FC<PageTemplateProps> = () => {
  return (
    <StyledPageTemplate>
      <ContentWrapper>
        <FormContainer>
          <FormContainerHeader>
            <MeteroidTitle />
            <PageTabs />
          </FormContainerHeader>
          <Outlet />
        </FormContainer>
        <Visual />
      </ContentWrapper>
    </StyledPageTemplate>
  )
}

export default PageTemplate

const PageTabs = () => {
  const location = useLocation()
  const { pathname, search } = location
  return (
    <Tabs>
      <Tab isActive={pathname === '/login'}>
        <Link to={`/login${search}`}>Login</Link>
      </Tab>
      <Tab isActive={pathname === '/registration'}>
        <Link to={`/registration${search}`}>Create account</Link>
      </Tab>
    </Tabs>
  )
}
