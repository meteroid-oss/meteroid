import { useMemo } from 'react'

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
      <div className="h-full self-center">
        <div>
          <h1 className="text-2xl">Good {timeOfDay}, user</h1>
          <span className="text-xs">{date}</span>
        </div>
      </div>
    </>
  )
}
