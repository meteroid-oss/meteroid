import { FunctionComponent } from 'react'

import Footer from './components/Footer'
import Header from './components/Header'
import Items from './components/Items'

export const NavigationBar: FunctionComponent = () => {
  return (
    <nav className="flex flex-col bg-card w-[55px] border-r justify-between items-center py-5">
      <Header />
      <Items />
      <Footer />
    </nav>
  )
}
