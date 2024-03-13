import { useMemo } from 'react'

import { DetailsSection } from '@/features/dashboard/sections/DetailsSection'
import { MrrSection } from '@/features/dashboard/sections/MrrSection'
import { TopSection } from '@/features/dashboard/sections/TopSection'
import { Card, Checkbox, Separator } from '@md/ui'
import { Heart } from 'lucide-react'

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
          <h1 className="text-2xl text-acc font-semibold">Good {timeOfDay}, Joe</h1>
          <span className="text-md font-medium text-muted-foreground">{date}</span>
        </div>
        <Separator />
        <Card variant="accent2">
          <div className="px-6 py-4">
            <div className="text-sm font-semibold pb-4">Complete your onboarding</div>
            <div>
              <div>
                <Checkbox disabled className="rounded-full border-none bg-success" checked />{' '}
                <span className="text-sm">Configure your pricing</span>
              </div>
              <div>
                <Checkbox disabled className="rounded-full" /> <span className="text-sm">???</span>
              </div>

              <div>
                <Checkbox disabled className="rounded-full" />{' '}
                <span className="text-sm">Profit !</span>
              </div>
            </div>
          </div>
        </Card>
        <TopSection />
        <MrrSection />
        <Separator />
        <DetailsSection />
        <Separator />
        <div className="h-10 text-center justify-center text-xs text-muted-foreground flex gap-1 ">
          <span>2024 © Meteroid /</span>
          <span className="flex items-baseline gap-1">
            Built with <Heart size="10" fill="red" strokeWidth={0} className="" /> in Europe /
          </span>
          <span>
            Open-source on{' '}
            <a href="https://go.meteroid.com/github" className="underline">
              Github
            </a>
          </span>
        </div>
      </div>
    </>
  )
}
