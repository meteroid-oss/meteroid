import { FontsPreload } from '@md/foundation'
import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import { Helmet } from 'react-helmet'

import { globalStyles } from '@/styles/typography'
import { App } from 'App'

import 'react-loading-skeleton/dist/skeleton.css'

import '@/styles/main.scss'
import '@md/foundation/styles'
// import '@md/ui/tailwind'

globalStyles()

createRoot(document.getElementById('root') as HTMLElement).render(
  <StrictMode>
    <Helmet>
      <FontsPreload />
    </Helmet>
    <App />
  </StrictMode>
)
