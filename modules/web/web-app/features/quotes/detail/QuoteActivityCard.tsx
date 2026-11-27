import { EntityActivityTimeline } from '@/features/activity/EntityActivityTimeline'
import { RailCard } from '@/features/quotes/detail/RailCard'

export const QuoteActivityCard = ({ quoteId }: { quoteId: string }) => (
  <RailCard title="Activity">
    <EntityActivityTimeline
      entityType="quote"
      entityId={quoteId}
      limit={5}
      emptyLabel="No activity recorded"
      timeline
    />
  </RailCard>
)
