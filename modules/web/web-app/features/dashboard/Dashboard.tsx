import { useMemo } from 'react'

import { DetailsSection } from '@/features/dashboard/sections/DetailsSection'
import { MrrSection } from '@/features/dashboard/sections/MrrSection'
import { TopSection } from '@/features/dashboard/sections/TopSection'

export const Dashboard = () => {
  const date = useMemo(() => {
    const today = new Date()
    const options = { weekday: 'long', year: 'numeric', month: 'long', day: 'numeric' } as const
    return today.toLocaleDateString('en-UK', options)
  }, [])

  // morning, afternoon or evening
  const timeOfDay = useMemo(() => {
    const hour = new Date().getHours()
    if (hour > 18 || hour < 4) {
      return 'evening'
    } else if (hour > 12) {
      return 'afternoon'
    } else {
      return 'morning'
    }
  }, [])

  return (
    <>
      <div className="h-full max-w-screen-xl xl:mx-auto self-center space-y-6 relative">
        <div>
          <h1 className="text-2xl">Good {timeOfDay}, Joe</h1>
          <span className="text-xs text-slate-1100">{date}</span>
        </div>
        <TopSection />
        <MrrSection />
        <DetailsSection />
      </div>
    </>
  )
}
