import { EmptyLogo } from '@/components/EmptyLogo'

export function ChartNoData({ error }: { error?: boolean }) {
  return (
    <div className=" h-full w-full flex flex-col gap-4 items-center justify-center ">
      {/* <div className="font-semibold text-sm text-center mb-4 bg-slate-100 rounded-xl p-4 z-10">
          {error ? 'error' : 'no data'}
        </div> */}
      <EmptyLogo size={40} />
      <div className=" text-sm font-medium ">{error ? 'error' : 'no data'}</div>
    </div>
  )
}
