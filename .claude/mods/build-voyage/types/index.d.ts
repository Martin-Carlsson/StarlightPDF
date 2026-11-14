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

declare module 'claude-code' {
  interface PluginState {
    'build-voyage': { voyage: Shaped<Voyage | null> }
  }
}
