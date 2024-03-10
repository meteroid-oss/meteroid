import { useTheme } from 'providers/ThemeProvider'

export function ChartNoData({ error }: { error?: boolean }) {
  const { isDarkMode } = useTheme()

  return (
    <div className=" h-full w-full flex flex-col gap-4 items-center justify-center ">
      {/* <div className="font-semibold text-sm text-center mb-4 bg-slate-100 rounded-xl p-4 z-10">
          {error ? 'error' : 'no data'}
        </div> */}
      {isDarkMode ? (
        <img src="/img/empty-dark.png" alt="no data" height={40} width={40} />
      ) : (
        <img src="/img/empty.png" alt="no data" height={40} width={40} />
      )}
      <div className="font-medium">{error ? 'error' : 'no data'}</div>
    </div>
  )
}
