import { useCallback, useMemo } from 'react'

import { changelog } from './changelog'
import { useUserPreferences } from './useUserPreferences'

export const useUpdates = () => {
  const { preferences, update } = useUserPreferences()
  const seenAt = preferences.changelogSeenAt

  const isUnread = useCallback((date: string) => !seenAt || date > seenAt, [seenAt])

  const unreadCount = useMemo(
    () => changelog().filter(entry => isUnread(entry.date)).length,
    [isUnread]
  )

  const newest = changelog()[0]?.date

  const markAllSeen = useCallback(() => {
    if (newest && newest !== seenAt) void update({ changelogSeenAt: newest })
  }, [newest, seenAt, update])

  return {
    entries: changelog(),
    unreadCount,
    hasUnread: unreadCount > 0,
    isUnread,
    markAllSeen,
  }
}
