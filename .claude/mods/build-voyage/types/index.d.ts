export type Check = { command: string; result: 'pass' | 'fail' | 'unknown' }

export type Voyage = {
  issue: string
  phase: string
  isBlocked: boolean
  /** The build finished: its checks passed and its pull request is open. */
  isDone: boolean
  checks: Check[]
  /** Minutes left of the 24 hours, or null while the clock is stopped. */
  fuelMinutes: number | null
  refusals: number
  pr: string
}

/** A /build that has not yet written its state. */
export type Launch = {
  /** The issue number from the arguments, or '' for none. */
  issue: string
  /** The state file's text when it ran. */
  state: string
  /** False once a turn ended with the state unchanged. */
  isLive: boolean
}

declare module 'claude-code' {
  interface PluginState {
    'build-voyage': { voyage: Shaped<Voyage | null>; launch: Shaped<Launch | null>; found: Shaped<string | false | null> }
  }
}
