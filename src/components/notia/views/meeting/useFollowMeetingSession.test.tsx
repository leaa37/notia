// @vitest-environment happy-dom
import { afterEach, describe, expect, it, vi } from 'vitest'
import { act, cleanup, renderHook } from '@testing-library/react'
import type { SpeechSessionState } from '../../../../services/speech/speechTypes'
import { FOLLOW_RETRY_MS, useFollowMeetingSession } from './useFollowMeetingSession'

type Status = SpeechSessionState['status']

const render = (attach: (id: string) => Promise<boolean>, refresh: () => Promise<unknown>, status: Status = 'idle') =>
  renderHook(
    ({ sessionId, current }: { sessionId: string | null; current: Status }) =>
      useFollowMeetingSession(attach, sessionId, current, refresh),
    { initialProps: { sessionId: 'meeting-1' as string | null, current: status } },
  )

describe('useFollowMeetingSession', () => {
  afterEach(() => {
    cleanup()
    vi.useRealTimers()
  })

  it('follows a running meeting while the voice hook is idle', async () => {
    const attach = vi.fn(async () => true)
    const refresh = vi.fn(async () => undefined)
    render(attach, refresh)
    await act(async () => undefined)

    expect(attach).toHaveBeenCalledWith('meeting-1')
    expect(refresh).not.toHaveBeenCalled()
  })

  it('reads the meeting again and retries when following fails', async () => {
    vi.useFakeTimers()
    const attach = vi.fn(async () => false)
    const refresh = vi.fn(async () => undefined)
    render(attach, refresh)
    await act(async () => undefined)
    expect(attach).toHaveBeenCalledTimes(1)
    expect(refresh).toHaveBeenCalledTimes(1)

    attach.mockResolvedValue(true)
    await act(async () => { vi.advanceTimersByTime(FOLLOW_RETRY_MS) })

    expect(attach).toHaveBeenCalledTimes(2)
    expect(refresh).toHaveBeenCalledTimes(1)
  })

  it('does nothing without a running meeting or while the hook follows one', async () => {
    const attach = vi.fn(async () => true)
    const refresh = vi.fn(async () => undefined)
    const view = render(attach, refresh, 'recording')
    view.rerender({ sessionId: null, current: 'idle' })
    await act(async () => undefined)

    expect(attach).not.toHaveBeenCalled()
  })
})
