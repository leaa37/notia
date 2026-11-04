import { useCallback, useEffect, useRef, useState } from 'react'
import type { SpeechSessionState } from '../../../../services/speech/speechTypes'

/** Wait before trying again to follow a session that did not answer. */
export const FOLLOW_RETRY_MS = 2_000

/**
 * Keeps a voice hook following the meeting session `sessionId` while the
 * backend reports it running and the hook is idle: after the view opens
 * again, after a dismissed error, or when the first try failed. A failed try
 * reads the meeting again (the backend settles a meeting whose session is
 * gone) and is repeated a little later.
 */
export function useFollowMeetingSession(
  attach: (sessionId: string) => Promise<boolean>,
  sessionId: string | null,
  status: SpeechSessionState['status'],
  refreshMeeting: () => Promise<unknown>,
) {
  const [retry, setRetry] = useState(0)
  const followingRef = useRef(false)
  const timerRef = useRef<number | null>(null)

  const follow = useCallback(async (id: string) => {
    const followed = await attach(id)
    if (!followed) void refreshMeeting()
    return followed
  }, [attach, refreshMeeting])

  useEffect(() => {
    if (!sessionId || status !== 'idle' || followingRef.current) return
    followingRef.current = true
    void follow(sessionId).then((followed) => {
      followingRef.current = false
      if (followed) return
      if (timerRef.current !== null) window.clearTimeout(timerRef.current)
      timerRef.current = window.setTimeout(() => {
        timerRef.current = null
        setRetry((count) => count + 1)
      }, FOLLOW_RETRY_MS)
    })
  }, [follow, retry, sessionId, status])

  useEffect(() => () => {
    if (timerRef.current !== null) window.clearTimeout(timerRef.current)
  }, [])
}
