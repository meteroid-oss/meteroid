import { Button } from '@md/ui'

export const NotFound = () => {
  return (
    <div className="flex w-full flex-col items-center justify-center gap-2">
      <div className="text-sm font-medium">Page not found</div>
      <p className="text-sm text-muted-foreground">This page does not exist.</p>
      <Button onClick={() => window.history.back()} size="sm" variant="secondary">
        Back
      </Button>
    </div>
  )
}
