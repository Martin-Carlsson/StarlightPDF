export type Check = { command: string; result: 'pass' | 'fail' | 'unknown' }

export type Voyage = {
  issue: string
  phase: string
  isBlocked: boolean
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
