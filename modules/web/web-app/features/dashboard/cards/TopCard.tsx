import { Skeleton } from '@ui2/components'
import { cn } from '@ui2/lib'
import { UserRoundIcon } from 'lucide-react'
import { Link } from 'react-router-dom'

interface TopCardProp {
  title: string
  loading: boolean
  values?: {
    name: string
    value: string
    logo?: string
    detailsPath?: string
  }[]
  className?: string
}

const colors = [
  'bg-red-700',
  'bg-purple-700',
  'bg-slate-500',
  'bg-indigo-700',
  'bg-blue-700',
  'bg-green-700',
  'bg-yellow-700',
]

const getColor = (key: string) => {
  // hash to get a proper random distribution
  const hash = key.split('').reduce((acc, char) => acc + char.charCodeAt(0), 0)
  return colors[hash % colors.length]
}

export const TopCard: React.FC<TopCardProp> = ({ title, values, className, loading }) => {
  return (
    <div className={cn(' overflow-y-auto h-[180px]  grow flex flex-col relative', className)}>
      <div className="text-sm font-semibold flex flex-row px-6 py-4 items-baseline w-full justify-between flex-grow">
        {title}
      </div>
      <div className="px-6 pb-4 space-y-3 relative">
        {loading ? (
          [1, 2, 3].map((_, index) => (
            <div key={index} className="flex flex-row gap-4 items-baseline text-xs justify-between">
              <Skeleton className="w-[40%]" height="1.2rem" />
              <Skeleton className="grow" height="1.2rem" />
            </div>
          ))
        ) : !values?.length ? (
          <>
            <div className="h-[74px] text-center  font-semibold text-sm ">no data</div>
          </>
        ) : (
          values.map((value, index) => (
            <div key={index} className="flex flex-row gap-4 items-baseline text-xs justify-between">
              <div className="flex flex-row items-center space-x-3 ">
                {value.logo ? (
                  <img src={value.logo} alt={value.name} />
                ) : (
                  <div
                    className={cn(
                      'p-1.5 flex items-center justify-center rounded-sm text-alternative-foreground',
                      getColor(value.name)
                    )}
                  >
                    <UserRoundIcon size={12} />
                  </div>
                )}

                {value.detailsPath ? (
                  <Link to={value.detailsPath}>
                    <span className="underline decoration-foreground decoration-dashed underline-offset-4">
                      {value.name}
                    </span>
                  </Link>
                ) : (
                  <span className="underline decoration-foreground decoration-dashed underline-offset-4">
                    {value.name}
                  </span>
                )}
              </div>
              <div className="text-sm">{value.value}</div>
            </div>
          ))
        )}
      </div>
    </div>
  )
}
