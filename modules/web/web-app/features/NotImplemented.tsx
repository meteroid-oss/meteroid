import { Button } from '@md/ui'

export const NotImplemented = () => {
  return (
    <div className="items-center justify-center flex flex-col gap-2 w-full">
      <div>Work in progress !</div>
      <Button onClick={() => window.history.back()} size="sm">
        Back
      </Button>
    </div>
  )
}
