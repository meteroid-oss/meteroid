import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'

import { env } from '@/lib/env'
import { App } from 'App'

import '@/styles/main.css'
import 'react-loading-skeleton/dist/skeleton.css'
import 'sonner/dist/styles.css'

// Inputs are recorded obscured and network payloads are not captured (tracker defaults).
async function startOpenReplay() {
  if (!env.openReplayProjectKey || !env.openReplayIngestPoint) return

  const { default: Tracker } = await import('@openreplay/tracker')
  const tracker = new Tracker({
    projectKey: env.openReplayProjectKey,
    ingestPoint: env.openReplayIngestPoint,
  })
  await tracker.start()
}

startOpenReplay().catch(error => console.error('OpenReplay failed to start', error))

createRoot(document.getElementById('root') as HTMLElement).render(
  <StrictMode>
    <App />
  </StrictMode>
)
