import { atom, read, update } from 'claude-code'
import type { Color, EngineInterface, Register, UiOpenResult } from 'claude-code'

import type { Agent, Build, Check, Launch, Run, Stale } from '../types'

// Reads what the build skill's hook keeps in .claude/build/ and draws it.
const PANE = 'build-voyage'
const DIR = '.claude/build'
const LIMIT_MINUTES = 24 * 60
const CAP = 30

const PHASES = [
  { phase: 'explore', name: 'Explore', status: 'Mapping the code and the spec.' },
  { phase: 'test', name: 'Test', status: 'Writing tests that fail until the issue is done.' },
  { phase: 'implement', name: 'Implement', status: 'Changing the code until every check passes.' },
  { phase: 'review', name: 'Review', status: 'Reviewing the diff against the issue.' },
  { phase: 'pr', name: 'Pull request', status: 'Opening the pull request.' },
  { phase: 'ci', name: 'CI', status: 'Waiting for CI to pass.' },
]

const build = atom({ plugin: 'build-voyage', key: 'voyage' } as const, null as Build | null, {
  shape: 'voyage-3',
})

const launch = atom({ plugin: 'build-voyage', key: 'launch' } as const, null as Launch | null, { shape: 'launch-1' })

const stale = atom({ plugin: 'build-voyage', key: 'stale' } as const, null as Stale | null, { shape: 'stale-1' })

const run = atom({ plugin: 'build-voyage', key: 'run' } as const, null as Run | null, { shape: 'run-1' })

const field = (text: string, name: string): string =>
  text.match(new RegExp(`^${name}: (.*)$`, 'm'))?.[1]?.trim() ?? ''

export const parse = (
  state: string,
  results: string,
  refusals: string,
  clock: string,
  finished: string,
  nowMs: number,
): Build => {
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
    minutesLeft: started > 0 ? Math.max(0, Math.min(LIMIT_MINUTES, LIMIT_MINUTES - used)) : null,
    refusals: Number(refusals.trim()) || 0,
    pr: field(state, 'pr'),
  }
}

const allPass = (one: Build | null): boolean =>
  one !== null && one.checks.length > 0 && one.checks.every(check => check.result === 'pass')

const mark = (check: Check): string => (check.result === 'pass' ? '✓' : check.result === 'fail' ? '✗' : '·')

type Row = {
  text: string
  color: Color
  isBold?: boolean
  isItalic?: boolean
  isDim?: boolean
  isFixed?: boolean
  bar?: { value: number; max: number }
}

const commas = (n: number): string => String(n).replace(/\B(?=(\d{3})+$)/g, ',')

const longest = (texts: string[]): number => texts.reduce((most, text) => Math.max(most, text.length), 0)

type Line = { mark: string; title: string; model: string; tokens: string }

const lines = (agents: Agent[], parentId?: string, depth = 0): Line[] =>
  agents
    .filter(agent => agent.parentId === parentId)
    .flatMap(agent => [
      {
        mark: agent.isRunning ? '⏳' : '✅',
        title: `${'  '.repeat(depth)}${agent.description}`,
        model: agent.model.replace(/^claude-/, ''),
        tokens: commas(agent.tokens),
      },
      ...lines(agents, agent.id, depth + 1),
    ])

// A title is cut to fit; the model and tokens never are.
const table = (tally: Run, columns?: number): Row[] => {
  const agents = lines(tally.agents)
  const all = [
    ...agents,
    { mark: '  ', title: 'Total', model: '', tokens: commas(tally.agents.reduce((sum, agent) => sum + agent.tokens, tally.mainTokens)) },
  ]
  const model = longest(agents.map(line => line.model))
  const tokens = longest(all.map(line => line.tokens))
  const title = Math.min(
    longest(all.map(line => line.title)),
    columns === undefined ? Infinity : Math.max(1, columns - 3 - 2 - model - 2 - tokens),
  )

  return all.map(line => ({
    text: `${line.mark} ${(line.title.length > title ? `${line.title.slice(0, title - 1)}…` : line.title).padEnd(title)}  ${line.model.padEnd(model)}  ${line.tokens.padStart(tokens)}`,
    color: 'text' as const,
    isFixed: true,
  }))
}

// Pane lines in groups; the text answer is the same lines.
const rows = (one: Build | null, tally: Run | null, columns?: number): Row[][] => {
  if (one === null) {
    return [
      [
        { text: 'Build', color: 'suggestion', isBold: true },
        { text: 'No build is running. Start one with /build and an issue number.', color: 'text', isDim: true },
      ],
    ]
  }
  const at = one.isDone ? PHASES.length : PHASES.findIndex(each => each.phase === one.phase)
  const passed = one.checks.filter(check => check.result === 'pass').length
  const left = one.minutesLeft
  const status = one.isDone
    ? 'CI passes; the pull request is ready to merge.'
    : one.isBlocked
      ? 'Waiting for your answer.'
      : (PHASES[at]?.status ?? 'Starting the build.')

  return [
    [{ text: one.issue === '' ? 'Build' : `Build #${one.issue}`, color: 'suggestion', isBold: true }],
    PHASES.map((each, index) => ({
      text: `${index < at ? '◉' : index === at ? '◎' : '○'} ${each.name}`,
      color: index < at ? 'success' : index === at ? 'claude' : 'inactive',
    })),
    [{ text: status, color: one.isBlocked ? 'warning' : 'text', isItalic: true }],
    [
      {
        text: `${passed} of ${one.checks.length} checks pass`,
        color: 'success',
        bar: { value: passed, max: Math.max(1, one.checks.length) },
      },
      ...one.checks.map(check => ({
        text: `${mark(check)} ${check.command}`,
        isFixed: true,
        color: check.result === 'pass' ? 'success' : check.result === 'fail' ? 'warning' : 'inactive',
      })),
    ],
    [
      ...(left === null
        ? []
        : [
            {
              text: `${Math.floor(left / 60)}h ${left % 60}m left before the build pauses`,
              color: left < 120 ? 'warning' : 'suggestion',
              bar: { value: left, max: LIMIT_MINUTES },
            },
          ]),
      {
        text: `${one.refusals} of ${CAP} stops refused`,
        color: one.refusals > 20 ? 'error' : 'permission',
        bar: { value: one.refusals, max: CAP },
      },
    ],
    ...(tally === null
      ? []
      : [
          [
            {
              text: `${Math.floor(tally.elapsedMs / 3_600_000)}h ${Math.floor(tally.elapsedMs / 60_000) % 60}m elapsed`,
              color: 'suggestion' as const,
            },
            ...table(tally, columns),
          ],
        ]),
    ...(one.pr === '' ? [] : [[{ text: one.pr, color: 'success' as const }]]),
  ]
}

const describe = (one: Build | null, tally: Run | null): string =>
  rows(one, tally)
    .flat()
    .map(row => row.text)
    .join('\n')

const open = async ($: EngineInterface): Promise<UiOpenResult> => {
  const number = /^\d+/.exec((await read($, build))?.issue ?? '')?.[0]

  return $.ui.open({ id: PANE, title: number === undefined ? 'Build' : `Build #${number}` })
}

// Polls only while a build runs; a tool call notices one that starts.
let timer: ReturnType<EngineInterface['clock']['every']> | undefined
let isFollowing = false

// Under the session's root, which follows a worktree the session enters.
const reader = async ($: EngineInterface) => {
  const dir = `${await $.session.root()}/${DIR}`

  return (name: string): Promise<string> => $.fs.read(`${dir}/${name}`).catch(() => '')
}

const look = async ($: EngineInterface): Promise<boolean> => {
  const file = await reader($)
  const state = await file('state')
  const launched = await read($, launch)
  const isLaunched = launched?.state === state

  if (launched !== null && !isLaunched) {
    await update($, launch, () => null)
  }
  const quiet = await read($, stale)
  const isQuiet = quiet === state

  if (quiet !== null && !isQuiet) {
    await update($, stale, () => null)
  }
  const next: Build | null = isLaunched
    ? { issue: launched.issue, phase: 'explore', isBlocked: false, isDone: false, checks: [], minutesLeft: null, refusals: 0, pr: '' }
    : state === ''
      ? null
      : parse(
          state,
          await file('results'),
          await file('refusals'),
          await file('started'),
          await file('done'),
          await $.clock.now(),
        )
  const isRunning = isLaunched || (next !== null && !next.isDone)

  if (isRunning && timer === undefined) {
    timer = $.clock.every(5000, () => void refresh($).catch(() => false))
  }
  if (!isRunning && timer !== undefined) {
    timer.cancel()
    timer = undefined
  }
  const now = await $.clock.now()

  await update($, run, one =>
    one === null || !one.isOpen ? one : { ...one, elapsedMs: now - one.startedMs, isOpen: isRunning },
  )
  const before = await read($, build)

  if (JSON.stringify(before) !== JSON.stringify(next)) {
    await update($, build, () => next)

    if (before === null && next !== null && !next.isDone) {
      void open($).catch(() => undefined)
    }
    if (!allPass(before) && allPass(next)) {
      $.ui.toast('Every check passes.')
    }
  }

  isFollowing = isRunning && !isQuiet

  return isFollowing
}

let queue: Promise<boolean> = Promise.resolve(false)

// One look at a time, after `step`; true while a closed pane should reopen.
const refresh = ($: EngineInterface, step: () => Promise<unknown> = async () => undefined): Promise<boolean> => {
  queue = queue.catch(() => false).then(async () => {
    await step()

    return look($)
  })

  return queue
}

const answer = async ($: EngineInterface, opened: UiOpenResult): Promise<string> =>
  opened.isPlaced
    ? 'Build pane opened.'
    : `${describe(await read($, build), await read($, run))}\n\nNo pane here: ${opened.reason}`

const stateNow = async ($: EngineInterface): Promise<string> => (await reader($))('state')

export const register: Register = on => {
  on('session.start', async ($, e, next) => {
    await $.command.register({ name: 'voyage', description: 'Show the progress of the current /build' })
    void refresh($, async () => {
      const state = await stateNow($)

      if (state !== '' && (await read($, build)) === null) {
        await update($, stale, () => state)
      }
    }).catch(() => false)

    return next(e)
  })

  on('tool.call', async ($, e, next) => {
    if (!isFollowing) {
      if (timer === undefined) {
        void refresh($).catch(() => false)
      }
    } else if ((await refresh($)) && !(await $.ui.panes()).some(pane => pane.id === PANE)) {
      await open($)
    }
    const isMove = e.tool === 'EnterWorktree' || e.tool === 'ExitWorktree'
    // Read first: a poll during the move may clear it.
    const pending = isMove ? await read($, launch) : null
    const result = await next(e)

    // The root moved: a pending launch belongs to the new root.
    if (isMove) {
      await refresh($, async () => {
        const state = await stateNow($)
        await update($, launch, () => (pending === null ? null : { ...pending, state }))
      }).catch(() => false)
    }

    return result
  }).catch((_, e, next) => next(e))

  on('command.run', { command: 'build' }, async ($, e, next) => {
    const number = /^\d+/.exec(e.args.trim())?.[0]
    await refresh($, async () => {
      const state = await stateNow($)
      await update($, stale, () => null)
      await update($, launch, () => ({ issue: number === undefined ? '' : String(Number(number)), state }))
      const startedMs = await $.clock.now()
      await update($, run, () => ({ startedMs, elapsedMs: 0, isOpen: true, mainTokens: 0, agents: [] }))
    })
    const opened = await open($)

    if (!opened.isPlaced) {
      $.ui.log(await answer($, opened))
    }

    return next(e)
  }).catch((_, e, next) => next(e))

  on('agent.spawn', async ($, e, next) => {
    const result = await next(e)
    const id = result.agentId

    if (id !== undefined) {
      const agent: Agent = { id, description: e.description, model: result.model, isRunning: true, tokens: 0 }
      await update($, run, one =>
        one?.isOpen === true && (e.parentAgentId === undefined || one.agents.some(each => each.id === e.parentAgentId))
          ? { ...one, agents: [...one.agents, e.parentAgentId === undefined ? agent : { ...agent, parentId: e.parentAgentId }] }
          : one,
      )
    }

    return result
  }).catch((_, e, next) => next(e))

  on('turn.start', async ($, e, next) => {
    const id = e.agentId

    await update($, run, one =>
      one?.isOpen === true && one.agents.some(agent => agent.id === id)
        ? { ...one, agents: one.agents.map(agent => (agent.id === id ? { ...agent, isRunning: true } : agent)) }
        : one,
    )

    return next(e)
  }).catch((_, e, next) => next(e))

  on('turn.complete', async ($, e, next) => {
    const id = e.agentId
    const u = e.usage
    const used = u === undefined ? 0 : u.input_tokens + u.output_tokens + u.cache_read_input_tokens + u.cache_creation_input_tokens

    await update($, run, one =>
      one === null || !one.isOpen
        ? one
        : id === undefined
          ? { ...one, mainTokens: one.mainTokens + used }
          : {
              ...one,
              agents: one.agents.map(agent =>
                agent.id === id ? { ...agent, isRunning: false, tokens: agent.tokens + used } : agent,
              ),
            },
    )

    return next(e)
  }).catch((_, e, next) => next(e))

  on('command.run', { command: 'voyage' }, async $ => {
    await refresh($)

    return { text: await answer($, await open($)) }
  })

  on('ui.render', { component: 'Pane', requestId: PANE }, async ($, e) => {
    const { Box, Text } = $.ui.resolve(e)
    const width = Math.max(1, e.props.bodyColumns)

    return (
      <Box flexDirection="column" gap={1}>
        {rows(await read($, build), await read($, run), width).map(group => (
          <Box flexDirection="column">
            {group.flatMap(row => {
              const filled = row.bar === undefined ? 0 : Math.round((Math.min(row.bar.max, Math.max(0, row.bar.value)) / row.bar.max) * width)

              return [
                <Text
                  color={row.color}
                  weight={row.isBold === true ? 'bold' : undefined}
                  italic={row.isItalic}
                  dimColor={row.isDim}
                  wrap={row.isFixed === true ? 'truncate-end' : undefined}
                >
                  {row.text}
                </Text>,
                ...(row.bar === undefined
                  ? []
                  : [
                      <Text color={row.color} wrap="truncate-end">
                        {'█'.repeat(filled) + '░'.repeat(width - filled)}
                      </Text>,
                    ]),
              ]
            })}
          </Box>
        ))}
      </Box>
    )
  })
}
