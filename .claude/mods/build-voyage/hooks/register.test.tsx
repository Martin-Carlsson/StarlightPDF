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

// Stands for the engine: the session's root, files (a name under the root's .claude/build/, or an
// absolute path), the clock at 100 000 s, its open panes, and the order of what reached it: `build` (the skill), `tool:open` or
// `tool:closed` (a tool call, and whether the pane was open), `log:<text>` (a transcript line).
const world = (
  on: On,
  files: Map<string, string> = new Map(),
  isPlaced: boolean = true,
  root: () => string = () => '/repo',
) => {
  const clock = mock.clock(on, { now: 100_000_000 })
  const reads = { count: 0 }
  const panes = new Set<string>()
  const events: string[] = []
  on('session.root', () => ({ value: root() }))
  on('command.register', () => ({ value: { command: 'voyage' } }))
  on('session.start', (_, e) => ({ cwd: e.cwd }))
  on('ui.toast', () => ({ value: undefined }))
  on('ui.open', (_, e) => {
    panes.add(e.id)

    return { value: isPlaced ? { isPlaced } : { isPlaced, reason: REASON } }
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
    reads.count += 1
    const value = [...files].find(([one]) => e.path === (one.startsWith('/') ? one : `${root()}/.claude/build/${one}`))?.[1]

    if (value === undefined) {
      throw new Error('ENOENT')
    }

    return { value }
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
    // A main model turn, `body` run inside it.
    turn: async (turnId: string, body: () => Promise<void>) => {
      await $.turn.start({ text: '', turnId })
      await body()
      await $.turn.complete({ answer: '', durationMs: 1000, isAborted: false, turnId, reason: 'answer' })
      await clock.settle()
    },
    // The user closes the pane.
    close: async () => {
      panes.delete(ID)
      await clock.settle()
    },
  })

  return { clock, reads, events, act, isOpen: () => panes.has(ID) }
}

// What the pane's Text matching `text` reads on each surface, cut to `part`.
const shown = async ($: Engine, text: RegExp, part: RegExp = /.*/): Promise<string[]> => {
  const all: string[] = []

  for (const surface of SURFACES) {
    const ui = await $.ui.mount({ ...PANE, surface })
    all.push((await ui.find({ type: 'Text', text }))?.text.match(part)?.[0] ?? '')
    await ui.unmount()
  }

  return all
}

// The current stage's mark and name on each surface.
const stage = ($: Engine): Promise<string[]> => shown($, /^◎/, /^◎ \S+/)

const every = (text: string): string[] => SURFACES.map(() => text)

test('a build shows its checks, stage and fuel on every surface', async ($, on) => {
  const { clock } = world(
    on,
    new Map([
      // Started one hour before now: 23 of the 24 hours are left.
      ['state', 'status: working\nissue: 12 Reject empty files\nphase: implement\ncheck: true\ncheck: false\ncheck: sh x\n'],
      ['started', '96400\n'],
      ['results', 'pass true\nfail false\ntree 4b825dc\n'],
      ['refusals', '3\n'],
    ]),
  )
  await $.session.start(START)
  await clock.settle()

  for (const surface of SURFACES) {
    const ui = await $.ui.mount({ ...PANE, surface })

    expect((await ui.find({ type: 'Text', text: /issue #/ }))?.text).toBe('Destination issue #12 Reject empty files')
    expect((await ui.find({ type: 'Text', text: /^★/ }))?.text).toBe('★ true')
    expect((await ui.find({ type: 'Text', text: /^☆/ }))?.text).toBe('☆ false')
    expect((await ui.find({ type: 'Text', text: /^·/ }))?.text).toBe('· sh x')
    expect((await ui.find({ type: 'Text', text: /Burn/ }))?.text).toMatch(/^◎ Burn/)
    expect((await ui.find({ type: 'Text', text: /Chart/ }))?.text).toBe('◉ Chart')
    expect((await ui.find({ type: 'Text', text: /Dock$/ }))?.text).toBe('○ Dock')
    expect((await ui.find({ type: 'Text', text: /Fuel/ }))?.text).toBe('Fuel 23h 0m until check-in')
    expect((await ui.find({ type: 'Text', text: /Constellation/ }))?.text).toBe('Constellation 1 of 3 stars lit')
    expect((await ui.find({ type: 'Text', text: /Shields/ }))?.text).toBe('Shields 3 of 30 bumps taken')
    await ui.unmount()
  }
})

test('no build shows the dry dock', async ($, on) => {
  world(on)
  await $.session.start(START)

  for (const surface of SURFACES) {
    const ui = await $.ui.mount({ ...PANE, surface })

    expect(await ui.find({ type: 'Text', text: /dry dock/ })).toBeDefined()
    await ui.unmount()
  }
})

test('an idle session does not poll; a tool call notices a build', async ($, on) => {
  const files = new Map<string, string>()
  const { clock, reads } = world(on, files)
  await $.session.start(START)
  await clock.settle()
  const atStart = reads.count
  await clock.advance(60_000)

  expect(reads.count).toBe(atStart)

  files.set('state', 'status: blocked\ncheck: true\n')
  await $.tool.call({ tool: 'Bash', command: 'ls' })
  await clock.settle()

  const ui = await $.ui.mount({ ...PANE, surface: 'terminal' })

  expect(await ui.find({ type: 'Text', text: /Holding orbit/ })).toBeDefined()
  expect(await ui.find({ type: 'Text', text: /uncharted/ })).toBeDefined()
  expect(await ui.find({ type: 'Text', text: /Fuel/ })).toBeUndefined()
  await ui.unmount()
})

test('where no pane can be shown, the command answers in text', async ($, on) => {
  const { clock, act } = world(
    on,
    new Map([
      ['state', 'status: working\nissue: 12 Reject empty files\nphase: test\ncheck: true\ncheck: false\n'],
      ['results', 'pass true\nfail false\n'],
    ]),
    false,
  )
  await $.session.start(START)
  await clock.settle()

  const { text } = await act($).run('voyage', '')

  expect(text).toContain('issue #12 Reject empty files')
  expect(text).toContain('Chart')
  expect(text).toContain('1 of 2 stars lit')
  expect(text).toContain('★ true')
  expect(text).toContain('☆ false')
  expect(text).toContain(REASON)
  expect(text).not.toContain('pane opened')
})

test('a running build is polled; a finished one is docked and no longer polled', async ($, on) => {
  const state = 'status: working\nissue: 12 Reject empty files\nphase: pr\ncheck: true\npr: https://example.test/1\n'
  const files = new Map([
    ['state', state],
    ['results', 'pass true\n'],
  ])
  const { clock, reads } = world(on, files)
  await $.session.start(START)
  await clock.settle()
  const running = reads.count
  await clock.advance(5000)

  expect(reads.count).toBeGreaterThan(running)

  // The hook copies the state to `done` when the build finishes.
  files.set('done', state)
  await clock.advance(5000)
  const finished = reads.count
  await clock.advance(60_000)

  expect(reads.count).toBe(finished)

  const ui = await $.ui.mount({ ...PANE, surface: 'terminal' })

  expect(await ui.find({ type: 'Text', text: /Docked\. Welcome home/ })).toBeDefined()
  expect((await ui.find({ type: 'Text', text: /Dock$/ }))?.text).toBe('◉ Dock')
  expect((await ui.find({ type: 'Text', text: /example\.test/ }))?.text).toBe('⚓ https://example.test/1')
  await ui.unmount()
})

test('/build 10 opens the pane before its first tool call', async ($, on) => {
  const { clock, events, act, isOpen } = world(on)
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
  expect(await stage($)).toEqual(every('◎ Scan'))

  files.set('state', 'status: working\nissue: 10 Some title\nphase: test\n')
  await call()

  expect(await shown($, /Destination/)).toEqual(every('Destination issue #10 Some title'))
  expect(await stage($)).toEqual(every('◎ Chart'))
})

test('/build 18 then entering a worktree still shows issue #18', async ($, on) => {
  const finished = 'status: working\nissue: 13 Some title\nphase: pr\ncheck: true\npr: https://example.test/13\n'
  const files = new Map([
    ['state', finished],
    ['done', finished],
    ['results', 'pass true\n'],
  ])
  let dir = '/main'
  on('tool.call', { tool: 'EnterWorktree' }, (_, e, next) => {
    dir = '/wt'
    files.clear()

    return next(e)
  })
  const { clock, act } = world(on, files, true, () => dir)
  const { run, call } = act($)
  await $.session.start(START)
  await clock.settle()
  await run('build', '18 but build it in a worktree')
  await $.tool.call({ tool: 'EnterWorktree', path: '/wt' })
  await call()

  expect(await shown($, /Destination/)).toEqual(every('Destination issue #18'))
  expect(await stage($)).toEqual(every('◎ Scan'))
})

test('/build whose arguments do not start with a number is bound for uncharted', async ($, on) => {
  const { clock, act, isOpen } = world(on)
  const { run, close } = act($)
  await $.session.start(START)
  await clock.settle()

  for (const args of ['', 'abc', 'abc 18', '#18']) {
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
  expect(await stage($)).toEqual(every('◎ Scan'))
  expect(await shown($, /Docked/)).toEqual(every(''))
})

test('/build opens a pane the user closed before it ran', async ($, on) => {
  const { clock, act, isOpen } = world(on)
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

test('a pane closed before the state is written reopens over a stale state from an earlier session', async ($, on) => {
  const files = new Map([['state', 'status: working\nissue: 12 Reject empty files\nphase: implement\n']])
  const { clock, act, isOpen } = world(on, files)
  const { run, call, close } = act($)
  await $.session.start(START)
  await clock.settle()
  await run('build', '10')
  await close()
  await call()

  expect(isOpen()).toBe(true)
})

test('a reload does not stop a running build reopening its pane', async ($, on) => {
  const files = new Map<string, string>()
  const { clock, act, isOpen } = world(on, files)
  const { run, call, close } = act($)
  await $.session.start(START)
  await clock.settle()
  await run('build', '10')
  files.set('state', 'status: working\nissue: 10 Some title\nphase: explore\n')
  await call()
  await $.session.start(START)
  await clock.settle()
  await close()
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

    expect(isOpen()).toBe(true)

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

test('a build asking before it writes its state reopens a closed pane in the next turn', async ($, on) => {
  const { clock, act, isOpen } = world(on)
  const { run, call, turn, close } = act($)
  await $.session.start(START)
  await clock.settle()
  await run('build', '10')
  await turn('t1', call)
  await close()
  await turn('t2', call)

  expect(isOpen()).toBe(true)
})

test('a build continuing in the next turn with its state unchanged reopens a closed pane', async ($, on) => {
  const files = new Map<string, string>()
  const { clock, act, isOpen } = world(on, files)
  const { run, call, turn, close } = act($)
  await $.session.start(START)
  await clock.settle()
  await run('build', '10')
  await turn('t1', async () => {
    files.set('state', 'status: blocked\nissue: 10 Some title\nphase: implement\n')
    await call()
  })
  await close()
  await turn('t2', call)

  expect(isOpen()).toBe(true)
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

test('a build in a worktree the session entered shows its own issue, stage and checks', async ($, on) => {
  const files = new Map<string, string>()
  let dir = '/main'
  // The engine moves the root during the tool; registered first, so above world's hook, which ends the chain.
  on('tool.call', { tool: 'EnterWorktree' }, (_, e, next) => {
    dir = '/wt'

    return next(e)
  })
  const { clock } = world(on, files, true, () => dir)
  files.set('/main/.claude/build/state', 'status: working\nissue: 12 Reject empty files\nphase: test\ncheck: true\ncheck: false\n')
  files.set('/main/.claude/build/results', 'pass true\nfail false\n')
  files.set('/wt/.claude/build/state', 'status: working\nissue: 19 Show every build\nphase: implement\ncheck: sh a\ncheck: sh b\n')
  files.set('/wt/.claude/build/results', 'pass sh a\n')
  await $.session.start(START)
  await clock.settle()

  expect(await shown($, /Destination/)).toEqual(every('Destination issue #12 Reject empty files'))
  expect(await stage($)).toEqual(every('◎ Chart'))
  expect(await shown($, /^☆/)).toEqual(every('☆ false'))

  await $.tool.call({ tool: 'EnterWorktree', path: '/wt' })
  await clock.settle()

  expect(await shown($, /Destination/)).toEqual(every('Destination issue #19 Show every build'))
  expect(await stage($)).toEqual(every('◎ Burn'))
  expect(await shown($, /^★/)).toEqual(every('★ sh a'))
  expect(await shown($, /^·/)).toEqual(every('· sh b'))
  expect(await shown($, /^☆/)).toEqual(every(''))
})
