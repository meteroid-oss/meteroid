import { Skeleton } from '@ui/components'
import { cn } from '@ui/lib'
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
export const TopCard: React.FC<TopCardProp> = ({ title, values, className, loading }) => {
  return (
    <div
      className={cn(
        'container overflow-y-auto h-[180px] w-[450px] min-w-[250px] border border-slate-500 !border-r-0  flex flex-col',
        className
      )}
    >
      <div className="text-sm font-semibold flex flex-row px-6 py-4 items-baseline w-full justify-between flex-grow">
        {title}
      </div>
      <div className="px-6 pb-4 space-y-3">
        {loading ? (
          [1, 2, 3].map((_, index) => (
            <div key={index} className="flex flex-row gap-4 items-baseline text-xs justify-between">
              <Skeleton containerClassName="w-[40%]" width="100%" height="1.2rem" />
              <Skeleton containerClassName="w-full" width="100%" height="1.2rem" />
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
                  <div className="p-1.5 bg-slate-500 flex items-center justify-center rounded-sm">
                    <UserRoundIcon size={12} />
                  </div>
                )}

                {value.detailsPath ? (
                  <Link to={value.detailsPath}>
                    <span className="underline decoration-slate-800 decoration-dashed underline-offset-4">
                      {value.value}
                    </span>
                  </Link>
                ) : (
                  <span className="underline decoration-slate-800 decoration-dashed underline-offset-4">
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
