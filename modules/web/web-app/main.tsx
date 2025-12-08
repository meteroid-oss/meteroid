import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'

import { App } from 'App'

import 'react-loading-skeleton/dist/skeleton.css'

import '@/styles/main.scss'
import 'sonner/dist/styles.css'

createRoot(document.getElementById('root') as HTMLElement).render(
  <StrictMode>
    <App />
  </StrictMode>
)
