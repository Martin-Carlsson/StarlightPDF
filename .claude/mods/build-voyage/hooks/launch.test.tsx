import { expect, mock, test } from 'claude-code/testing'
import type { Engine } from 'claude-code/testing'
import type { On } from 'claude-code'

const ID = 'build-voyage'
const PANE = {
  plugin: ID,
  component: 'Pane',
  requestId: ID,
  props: {
    title: 'Starlight voyage',
    isFocused: false,
    bodyColumns: 40,
    placement: 'dock',
    scroll: { offset: 0, bodyRows: 40 },
    view: {},
  },
} as const
const START = { cwd: '/', surface: 'desktop', isInteractive: true } as const
const SURFACES = ['terminal', 'desktop', 'vscode', 'mobile'] as const
const REASON = 'no surface places panes'

// Stands for the engine: the files of .claude/build/, its open panes, and the
// order of what reached it: `build` (the skill), `tool:open` or `tool:closed`
// (a tool call, and whether the pane was open), `log:<text>` (a transcript line).
const world = (on: On, files: Map<string, string>, isPlaced: boolean = true) => {
  const clock = mock.clock(on, { now: 100_000_000 })
  const panes = new Set<string>()
  const events: string[] = []
  on('command.register', () => ({ value: { command: 'voyage' } }))
  on('session.start', (_, e) => ({ cwd: e.cwd }))
  on('ui.toast', () => ({ value: undefined }))
  on('ui.open', (_, e) => {
    panes.add(e.id)

    return { value: isPlaced ? { isPlaced } : { isPlaced, reason: REASON } }
  })
  on('ui.close', (_, e) => {
    panes.delete(e.id)

    return { value: undefined }
  })
  on('ui.panes', () => ({
    value: [...panes].map(id => ({ id, title: id, isShown: isPlaced, isFocused: false, isPlaced })),
  }))
  on('ui.log', (_, e) => {
    if (e.to === 'transcript') {
      events.push(`log:${e.text}`)
    }

    return { value: undefined }
  })
  on('command.run', { command: 'build' }, () => {
    events.push('build')

    return { text: '' }
  })
  on('turn.start', (_, e) => ({ turnId: e.turnId }))
  on('turn.complete', (_, e) => ({ text: e.answer }))
  on('tool.call', () => {
    events.push(panes.has(ID) ? 'tool:open' : 'tool:closed')

    return { result: {} }
  })
  on('fs.read', (_, e) => {
    const name = [...files.keys()].find(one => e.path.endsWith(`.claude/build/${one}`))

    if (name === undefined) {
      throw new Error('ENOENT')
    }

    return { value: files.get(name) ?? '' }
  })

  const act = ($: Engine) => ({
    run: async (command: string, args: string) => {
      const ran = await $.command.run({
        command,
        args,
        origin: { kind: 'composer' },
        presentation: { isFullscreen: false, columns: 80 },
      })
      await clock.settle()

      return ran
    },
    call: async () => {
      await $.tool.call({ tool: 'Bash', command: 'ls' })
      await clock.settle()
    },
    // A model turn, `body` run inside it; a subagent's with `agentId`.
    turn: async (turnId: string, body: () => Promise<void>, agentId?: string) => {
      const agent = agentId === undefined ? {} : { agentId }
      await $.turn.start({ text: '', turnId, ...agent })
      await body()
      await $.turn.complete({ answer: '', durationMs: 1000, isAborted: false, turnId, reason: 'answer', ...agent })
      await clock.settle()
    },
    // The user's close mark, as the engine's record sees it; a test's $ has no ui.close.
    close: async () => {
      panes.delete(ID)
      await clock.settle()
    },
  })

  return { clock, events, act, isOpen: () => panes.has(ID) }
}

// What the pane's Text matching `text` reads on each surface.
const shown = async ($: Engine, text: RegExp): Promise<string[]> => {
  const all: string[] = []

  for (const surface of SURFACES) {
    const ui = await $.ui.mount({ ...PANE, surface })
    all.push((await ui.find({ type: 'Text', text }))?.text ?? '')
    await ui.unmount()
  }

  return all
}

const every = (text: string): string[] => SURFACES.map(() => text)

test('/build 10 opens the pane before its first tool call', async ($, on) => {
  const { clock, events, act, isOpen } = world(on, new Map())
  const { run, call } = act($)
  await $.session.start(START)
  await clock.settle()

  expect(isOpen()).toBe(false)

  await run('build', '10')

  expect(isOpen()).toBe(true)

  await call()

  expect(events).toContain('build')
  expect(events.find(one => one.startsWith('tool:'))).toBe('tool:open')
})

test('until the build writes its state, the pane shows issue #10 at the first stage', async ($, on) => {
  const files = new Map<string, string>()
  const { clock, act } = world(on, files)
  const { run, call } = act($)
  await $.session.start(START)
  await clock.settle()
  await run('build', '10')
  await call()
  await clock.advance(60_000)

  expect(await shown($, /Destination/)).toEqual(every('Destination issue #10'))
  expect(await shown($, /you are here/)).toEqual(every('◎ Scan  ⟵ you are here'))

  files.set('state', 'status: working\nissue: 10 Some title\nphase: test\n')
  await call()

  expect(await shown($, /Destination/)).toEqual(every('Destination issue #10 Some title'))
  expect(await shown($, /you are here/)).toEqual(every('◎ Chart  ⟵ you are here'))
})

test('/build with no number or a non-number is bound for uncharted', async ($, on) => {
  const { clock, act, isOpen } = world(on, new Map())
  const { run, close } = act($)
  await $.session.start(START)
  await clock.settle()

  for (const args of ['', 'abc']) {
    await run('build', args)

    expect(isOpen()).toBe(true)
    expect(await shown($, /Destination/)).toEqual(every('Destination uncharted'))
    await close()
  }
})

test('/build 10 shows issue #10, not a finished build still on disk', async ($, on) => {
  const old = 'status: working\nissue: 12 Reject empty files\nphase: pr\ncheck: true\npr: https://example.test/1\n'
  const files = new Map([
    ['state', old],
    ['done', old],
    ['results', 'pass true\n'],
  ])
  const { clock, act } = world(on, files)
  const { run, call } = act($)
  await $.session.start(START)
  await clock.settle()
  await run('build', '10')
  await call()

  expect(await shown($, /Destination/)).toEqual(every('Destination issue #10'))
  expect(await shown($, /you are here/)).toEqual(every('◎ Scan  ⟵ you are here'))
  expect(await shown($, /Docked/)).toEqual(every(''))
})

test('/build opens a pane the user closed before it ran', async ($, on) => {
  const { clock, act, isOpen } = world(on, new Map())
  const { run, close } = act($)
  await $.session.start(START)
  await clock.settle()
  await run('voyage', '')

  expect(isOpen()).toBe(true)

  await close()

  expect(isOpen()).toBe(false)

  await run('build', '10')

  expect(isOpen()).toBe(true)
})

test('a pane closed during a build opens again on the next tool call', async ($, on) => {
  const files = new Map<string, string>()
  const { clock, act, isOpen } = world(on, files)
  const { run, call, close } = act($)
  await $.session.start(START)
  await clock.settle()
  await run('build', '10')
  files.set('state', 'status: working\nissue: 10 Some title\nphase: explore\n')
  await call()

  expect(isOpen()).toBe(true)

  await close()

  expect(isOpen()).toBe(false)

  await call()

  expect(isOpen()).toBe(true)
})

test('a pane closed after a build finished or was cancelled stays closed', async ($, on) => {
  const files = new Map<string, string>()
  const { clock, act, isOpen } = world(on, files)
  const { run, call, close } = act($)
  const endings = [
    { issue: '10', end: () => files.set('done', files.get('state') ?? '') },
    { issue: '11', end: () => files.delete('state') },
  ]
  await $.session.start(START)
  await clock.settle()

  for (const { issue, end } of endings) {
    await run('build', issue)
    files.set('state', `status: working\nissue: ${issue} Some title\nphase: pr\npr: https://example.test/${issue}\n`)
    await call()
    await close()
    await call()

    // While the build runs, a tool call opens it again.
    expect(isOpen()).toBe(true)

    // The end of the turn ends it, no tool call after; the 5 s poll notices.
    end()
    await clock.advance(5000)
    await close()
    await call()
    await clock.advance(60_000)

    expect(isOpen()).toBe(false)
  }
})

test('where no pane can be drawn, /build prints the voyage before its first tool call', async ($, on) => {
  const { clock, events, act } = world(on, new Map(), false)
  const { run, call } = act($)
  await $.session.start(START)
  await clock.settle()
  await run('build', '10')
  await call()

  const first = events.findIndex(one => one.startsWith('tool:'))
  const logged = events
    .slice(0, first)
    .find(one => one.startsWith('log:'))
    ?.slice(4)

  expect(first).toBeGreaterThan(-1)
  expect(logged).toContain('✦ Destination: issue #10')
  expect(logged).toBe((await run('voyage', '')).text)
})

test('a /build that stops before writing its state stops reopening the pane', async ($, on) => {
  const { clock, act, isOpen } = world(on, new Map())
  const { run, call, turn, close } = act($)
  await $.session.start(START)
  await clock.settle()
  await run('build', '10')
  await turn('t1', call)

  expect(await shown($, /Destination/)).toEqual(every('Destination issue #10'))
  expect(await shown($, /you are here/)).toEqual(every('◎ Scan  ⟵ you are here'))

  await close()
  await turn('t2', call)
  await clock.advance(60_000)

  expect(isOpen()).toBe(false)
})

test('a stale unfinished state from an earlier session does not reopen a closed pane', async ($, on) => {
  const files = new Map([['state', 'status: working\nissue: 12 Reject empty files\nphase: implement\n']])
  const { clock, act, isOpen } = world(on, files)
  const { call, close } = act($)
  await $.session.start(START)
  await clock.settle()
  await close()
  await call()
  await call()
  await clock.advance(60_000)

  expect(isOpen()).toBe(false)
})
