import { EmptyLogo } from '@/components/EmptyLogo'

export function ChartNoData({ error }: { error?: boolean }) {
  return (
    <div className=" h-full w-full flex flex-col gap-4 items-center justify-center ">
      <EmptyLogo size={40} />
      <div className=" text-sm font-medium ">{error ? 'error' : 'no data'}</div>
    </div>
  )
}
