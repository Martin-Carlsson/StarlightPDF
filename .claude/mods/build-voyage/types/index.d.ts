export type Check = { command: string; result: 'pass' | 'fail' | 'unknown' }

export type Build = {
  issue: string
  phase: string
  isBlocked: boolean
  /** The build finished: its checks passed and its pull request is open. */
  isDone: boolean
  checks: Check[]
  /** Minutes left of the 24 hours, or null while the clock is stopped. */
  minutesLeft: number | null
  refusals: number
  pr: string
}

/** A /build that has not yet changed the state file. */
export type Launch = {
  /** The issue number from its arguments, or ''. */
  issue: string
  /** The state file's text when it ran. */
  state: string
}

/** State text found at session start; no reopen until it changes or /build runs. */
export type Stale = string

declare module 'claude-code' {
  interface PluginState {
    'build-voyage': { voyage: Shaped<Build | null>; launch: Shaped<Launch | null>; stale: Shaped<Stale | null> }
  }
}
