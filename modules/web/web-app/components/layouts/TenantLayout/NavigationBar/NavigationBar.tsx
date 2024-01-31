import { FunctionComponent } from 'react'

import { StyledNavigationBar } from './NavigationBar.styled'
import Footer from './components/Footer'
import Header from './components/Header'
import Items from './components/Items'

export const NavigationBar: FunctionComponent = () => {
  return (
    <StyledNavigationBar>
      <Header />
      <Items />
      <Footer />
    </StyledNavigationBar>
  )
}
