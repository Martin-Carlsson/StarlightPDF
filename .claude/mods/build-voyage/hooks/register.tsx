import { atom, read, update } from 'claude-code'
import type { EngineInterface, Register } from 'claude-code'

import type { Check, Voyage } from '../types'

// Reads what the build skill's hook keeps in .claude/build/ and draws it.
const PANE = 'build-voyage'
const TITLE = 'Starlight voyage'
const DIR = '.claude/build'
const LIMIT_MINUTES = 24 * 60
const CAP = 30

const STAGES = [
  { phase: 'explore', name: 'Scan', log: 'Probes are sweeping the nebula.' },
  { phase: 'test', name: 'Chart', log: 'Charting the stars we must light.' },
  { phase: 'implement', name: 'Burn', log: 'Main engines burning. Hold on to your teacup.' },
  { phase: 'review', name: 'Inspect', log: 'Inspection drones are circling the hull.' },
  { phase: 'pr', name: 'Dock', log: 'On final approach to the docking ring.' },
]

const voyage = atom({ plugin: 'build-voyage', key: 'voyage' } as const, null as Voyage | null, {
  shape: 'voyage-2',
})

const field = (text: string, name: string): string =>
  text.match(new RegExp(`^${name}: (.*)$`, 'm'))?.[1]?.trim() ?? ''

export const parse = (
  state: string,
  results: string,
  refusals: string,
  clock: string,
  finished: string,
  nowMs: number,
): Voyage => {
  const done = results.split('\n').filter(line => line !== '')
  const checks: Check[] = state
    .split('\n')
    .filter(line => line.startsWith('check: '))
    .map(line => line.slice(7))
    .map(command => ({
      command,
      result: done.includes(`pass ${command}`) ? 'pass' : done.includes(`fail ${command}`) ? 'fail' : 'unknown',
    }))
  const started = Number(clock.trim()) || 0
  const used = Math.floor((nowMs / 1000 - started) / 60)

  return {
    issue: field(state, 'issue'),
    phase: field(state, 'phase'),
    isBlocked: field(state, 'status') === 'blocked',
    isDone: finished === state,
    checks,
    fuelMinutes: started > 0 ? Math.max(0, Math.min(LIMIT_MINUTES, LIMIT_MINUTES - used)) : null,
    refusals: Number(refusals.trim()) || 0,
    pr: field(state, 'pr'),
  }
}

const lit = (one: Voyage | null): boolean =>
  one !== null && one.checks.length > 0 && one.checks.every(check => check.result === 'pass')

const star = (check: Check): string => (check.result === 'pass' ? '★' : check.result === 'fail' ? '☆' : '·')

// The pane in words, for a surface that shows no pane.
const describe = (one: Voyage | null): string => {
  if (one === null) {
    return 'The ship is in dry dock. Launch with /build and an issue number.'
  }
  const passed = one.checks.filter(check => check.result === 'pass').length
  const stage = STAGES.find(each => each.phase === one.phase)?.name ?? 'Pre-flight'
  const fuel = one.fuelMinutes

  return [
    `✦ Destination: ${one.issue === '' ? 'uncharted' : `issue #${one.issue}`}`,
    `Stage: ${one.isDone ? 'Docked' : one.isBlocked ? 'Holding orbit, waiting for you' : stage}`,
    `Constellation: ${passed} of ${one.checks.length} stars lit`,
    ...one.checks.map(check => `  ${star(check)} ${check.command}`),
    ...(fuel === null ? [] : [`Fuel: ${Math.floor(fuel / 60)}h ${fuel % 60}m until check-in`]),
    `Shields: ${one.refusals} of ${CAP} bumps taken`,
    ...(one.pr === '' ? [] : [`Docked: ${one.pr}`]),
  ].join('\n')
}

// Polls only while a build runs; a tool call notices one that starts.
let timer: ReturnType<EngineInterface['clock']['every']> | undefined

const refresh = async ($: EngineInterface): Promise<void> => {
  const file = (name: string) => $.fs.read(`${DIR}/${name}`).catch(() => '')
  const state = await file('state')
  const next =
    state === ''
      ? null
      : parse(
          state,
          await file('results'),
          await file('refusals'),
          await file('started'),
          await file('done'),
          await $.clock.now(),
        )
  const isRunning = next !== null && !next.isDone

  if (isRunning && timer === undefined) {
    timer = $.clock.every(5000, () => void refresh($))
  }
  if (!isRunning && timer !== undefined) {
    timer.cancel()
    timer = undefined
  }
  const before = await read($, voyage)

  if (JSON.stringify(before) === JSON.stringify(next)) {
    return
  }
  await update($, voyage, () => next)

  if (before === null && next !== null) {
    void $.ui.open({ id: PANE, title: TITLE })
  }
  if (!lit(before) && lit(next)) {
    $.ui.toast('✦ Every star is lit. Prepare for docking. ✦')
  }
}

export const register: Register = on => {
  on('session.start', async ($, e, next) => {
    await $.command.register({ name: 'voyage', description: 'Show the progress of the current /build' })
    void refresh($)

    return next(e)
  })

  on('tool.call', async ($, e, next) => {
    const ran = await next(e)

    if (timer === undefined) {
      void refresh($)
    }

    return ran
  })

  on('command.run', { command: 'voyage' }, async $ => {
    await refresh($)
    const opened = await $.ui.open({ id: PANE, title: TITLE })

    return {
      text: opened.isPlaced
        ? 'Voyage pane opened.'
        : `${describe(await read($, voyage))}\n\nNo pane here: ${opened.reason}`,
    }
  })

  on('ui.render', { component: 'Pane', requestId: PANE }, async ($, e) => {
    const { Box, Text, Progress } = $.ui.resolve(e)
    const one = await read($, voyage)

    if (one === null) {
      return (
        <Box flexDirection="column" gap={1}>
          <Text color="suggestion" weight="bold">
            ✦ · ˚ STARLIGHT VOYAGE ˚ · ✦
          </Text>
          <Text dimColor>The ship is in dry dock. Launch with /build and an issue number.</Text>
        </Box>
      )
    }

    const at = one.isDone ? STAGES.length : STAGES.findIndex(stage => stage.phase === one.phase)
    const passed = one.checks.filter(check => check.result === 'pass').length
    const fuel = one.fuelMinutes
    const log = one.isDone
      ? 'Docked. Welcome home, crew.'
      : one.isBlocked
        ? 'Holding orbit. Awaiting word from mission control. That is you.'
        : (STAGES[at]?.log ?? 'Pre-flight checks. Counting snacks.')

    return (
      <Box flexDirection="column" gap={1}>
        <Box flexDirection="column">
          <Text color="suggestion" weight="bold">
            ✦ · ˚ STARLIGHT VOYAGE ˚ · ✦
          </Text>
          <Text>
            <Text dimColor>Destination </Text>
            {one.issue === '' ? 'uncharted' : `issue #${one.issue}`}
          </Text>
        </Box>

        <Box flexDirection="column">
          <Text dimColor>FLIGHT PLAN</Text>
          {STAGES.map((stage, index) => (
            <Text color={index < at ? 'success' : index === at ? 'claude' : 'inactive'}>
              {index < at ? '◉' : index === at ? '◎' : '○'} {stage.name}
              {index === at && !one.isBlocked ? '  ⟵ you are here' : ''}
            </Text>
          ))}
        </Box>

        <Box flexDirection="column">
          <Progress
            label="Constellation"
            value={passed}
            max={Math.max(1, one.checks.length)}
            valueText={`${passed} of ${one.checks.length} stars lit`}
            color="success"
          />
          {one.checks.map(check => (
            <Text color={check.result === 'pass' ? 'success' : check.result === 'fail' ? 'warning' : 'inactive'} wrap="truncate-end">
              {star(check)} {check.command}
            </Text>
          ))}
        </Box>

        <Box flexDirection="column">
          {fuel !== null && (
            <Progress
              label="Fuel"
              value={fuel}
              max={LIMIT_MINUTES}
              valueText={`${Math.floor(fuel / 60)}h ${fuel % 60}m until check-in`}
              color={fuel < 120 ? 'warning' : 'suggestion'}
            />
          )}
          <Progress
            label="Shields"
            value={CAP - one.refusals}
            max={CAP}
            valueText={`${one.refusals} of ${CAP} bumps taken`}
            color={one.refusals > 20 ? 'error' : 'permission'}
          />
        </Box>

        <Box flexDirection="column">
          <Text dimColor>SHIP'S LOG</Text>
          <Text color={one.isBlocked ? 'warning' : 'text'} italic>
            {log}
          </Text>
          {one.pr !== '' && <Text color="success">⚓ {one.pr}</Text>}
        </Box>
      </Box>
    )
  })
}
