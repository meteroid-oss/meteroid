import { Logo } from '@md/foundation'
import { Link, useLocation } from 'react-router-dom'

import {
  ContentWrapper,
  FormContainer,
  FormContainerHeader,
  StyledPageTemplate,
  Tab,
  Tabs,
  Visual,
} from '@/features/auth/components/PageTemplate/PageTemplate.styled'
import { useTheme } from 'providers/ThemeProvider'

import type { FC, ReactNode } from 'react'

interface PageTemplateProps {
  form: ReactNode
}

const PageTemplate: FC<PageTemplateProps> = ({ form }) => {
  const { isDarkMode } = useTheme()
  const location = useLocation()
  const { pathname, search } = location

  return (
    <StyledPageTemplate>
      <ContentWrapper>
        <FormContainer>
          <FormContainerHeader>
            <Logo isDarkMode={isDarkMode} />

            <Tabs>
              <Tab isActive={pathname === '/login'}>
                <Link to={`/login${search}`}>Login</Link>
              </Tab>
              <Tab isActive={pathname === '/registration'}>
                <Link to={`/registration${search}`}>Create account</Link>
              </Tab>
            </Tabs>
          </FormContainerHeader>
          {form}
        </FormContainer>
        <Visual />
      </ContentWrapper>
    </StyledPageTemplate>
  )
}

export default PageTemplate
