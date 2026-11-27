import { Skeleton, cn } from '@md/ui'
import { UserRoundIcon } from 'lucide-react'
import { Link } from 'react-router'

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

// Neutral initial tiles, as in the Home reference (no per-customer colours).
const AVATAR = 'bg-foreground/10 text-foreground/70'

export const TopCard: React.FC<TopCardProp> = ({ title, values, className, loading }) => {
  return (
    <div className={cn(' overflow-y-auto h-[180px]  grow flex flex-col relative', className)}>
      <div className="text-[13px] leading-none tracking-[-0.13px] text-foreground/60 flex flex-row px-5 pt-5 pb-4 items-baseline w-full justify-between grow">
        {title}
      </div>
      <div className="px-5 pb-5 space-y-3 relative">
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
                  <div className={cn('p-1.5 flex items-center justify-center rounded-sm', AVATAR)}>
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
