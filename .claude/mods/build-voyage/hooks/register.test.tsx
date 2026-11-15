import { expect, mock, test } from 'claude-code/testing'
import type { Engine } from 'claude-code/testing'
import type { On, TurnUsage } from 'claude-code'

const ID = 'build-voyage'
const PANE = {
  plugin: ID,
  component: 'Pane',
  requestId: ID,
  props: {
    title: 'Build',
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

type Surface = (typeof SURFACES)[number]

// Stands for the engine: the session's root, files (a name under the root's .claude/build/, or an
// absolute path), the clock at 100 000 s, its open panes, and `events` in the order they came:
// `build` (the skill), `tool:open` or `tool:closed` (a tool call, by pane), `log:<text>`.
const world = (
  on: On,
  files: Map<string, string> = new Map(),
  isPlaced: boolean = true,
  root: () => string = () => '/repo',
  model: string = 'claude-fable-5-2',
) => {
  const clock = mock.clock(on, { now: 100_000_000 })
  const reads = { count: 0 }
  const panes = new Set<string>()
  const events: string[] = []
  on('session.root', () => ({ value: root() }))
  on('session.model', () => ({ value: model }))
  on('command.register', () => ({ value: { command: 'voyage' } }))
  on('session.start', (_, e) => ({ cwd: e.cwd }))
  // Toasts and pane titles received, and whether panes are placed.
  const seen = { toasts: [] as string[], titles: [] as string[], isPlaced }
  on('ui.toast', (_, e) => {
    seen.toasts.push(e.text)

    return { value: undefined }
  })
  on('ui.open', (_, e) => {
    seen.titles.push(e.title ?? '')
    panes.add(e.id)

    return { value: seen.isPlaced ? { isPlaced: true } : { isPlaced: false, reason: REASON } }
  })
  on('ui.panes', () => ({
    value: [...panes].map(id => ({ id, title: id, isShown: seen.isPlaced, isFocused: false, isPlaced: seen.isPlaced })),
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
  let spawned = 0
  on('agent.spawn', (_, e) => ({ model: e.model ?? 'claude-haiku-5-5', agentId: `agent-${(spawned += 1)}` }))
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

  return { clock, reads, events, seen, act, isOpen: () => panes.has(ID) }
}

const every = <T,>(value: T): T[] => SURFACES.map(() => value)

const eachSurface = async <T,>(read: (surface: Surface) => Promise<T>): Promise<T[]> => {
  const all: T[] = []

  for (const surface of SURFACES) {
    all.push(await read(surface))
  }

  return all
}

type Tree = { type: string; props: Record<string, unknown>; children?: unknown }

const isTree = (node: unknown): node is Tree =>
  typeof node === 'object' &&
  node !== null &&
  'type' in node &&
  typeof node.type === 'string' &&
  'props' in node &&
  typeof node.props === 'object' &&
  node.props !== null

const kids = (node: Tree): unknown[] => (Array.isArray(node.children) ? node.children : [])

const textOf = (node: unknown): string => {
  if (typeof node === 'string') {
    return node
  }

  if (!isTree(node)) {
    return ''
  }

  const label = node.props['label']

  if (node.children === undefined && typeof label === 'string') {
    return label
  }

  return kids(node).map(textOf).join('')
}

// A Text, a Link or a row Box is one row (the kit has no layout).
const rowsIn = (node: unknown): string[] => {
  if (!isTree(node)) {
    return []
  }

  if (node.type === 'Text' || node.type === 'Link' || (node.type === 'Box' && node.props['flexDirection'] === 'row')) {
    return [textOf(node)]
  }

  return kids(node).flatMap(rowsIn)
}

const paneRows = async ($: Engine, surface: Surface, columns: number = PANE.props.bodyColumns): Promise<string[]> => {
  const ui = await $.ui.mount({ ...PANE, surface, props: { ...PANE.props, bodyColumns: columns } })
  const all = rowsIn(await ui.drawn())
  await ui.unmount()

  return all.filter(one => one.trim() !== '')
}

const rowsEach = ($: Engine, columns?: number): Promise<string[][]> => eachSurface(surface => paneRows($, surface, columns))

const headEach = async ($: Engine, columns?: number): Promise<(string | undefined)[]> =>
  (await rowsEach($, columns)).map(rows => rows[0])

const currentOf = (rows: string[]): string[] =>
  rows.join('\n').match(/◉ (Explore|Test|Implement|Review|Pull request|CI|Retro)(?=  |$)/gm) ?? []

const currentEach = async ($: Engine, columns?: number): Promise<string[][]> => (await rowsEach($, columns)).map(currentOf)

const tableOf = (rows: string[]): string[] => {
  const from = rows.findIndex(one => /^ {2}Agent( |$)/.test(one))
  const to = rows.findIndex(one => /^ {2}Total( |$)/.test(one))

  return from < 0 || to < from ? [] : rows.slice(from, to + 1)
}

// `left` at the left edge, `right` at the right edge of `columns`.
const header = (left: string, right: string, columns: number): string =>
  left + ' '.repeat(columns - left.length - right.length) + right

const lines = async ($: Engine, surface: Surface): Promise<string[]> => {
  const ui = await $.ui.mount({ ...PANE, surface })
  const all = (await ui.findAll({ type: 'Text' })).map(one => one.text)
  await ui.unmount()

  return all
}

const coloursOf = ($: Engine, text: RegExp): Promise<unknown[]> =>
  eachSurface(async surface => {
    const ui = await $.ui.mount({ ...PANE, surface, props: { ...PANE.props, bodyColumns: 62 } })
    const colour = (await ui.find({ type: 'Text', text }))?.props['color']
    await ui.unmount()

    return colour
  })

const linksEach = ($: Engine): Promise<{ href: unknown; text: string }[][]> =>
  eachSurface(async surface => {
    const ui = await $.ui.mount({ ...PANE, surface, props: { ...PANE.props, bodyColumns: 62 } })
    const all = (await ui.findAll({ type: 'Link' })).map(one => ({ href: one.props['href'], text: one.text }))
    await ui.unmount()

    return all
  })

const TITLE = 'Keep files, diffs and logs out of the main session of `/build`'
const CHECKS = [
  "grep -q 'You read no source file, test file, diff or log you never open' .claude/skills/build/SKILL.md",
  "grep -q 'a subagent reads its log' .claude/skills/build/SKILL.md",
  "grep -q 'never file contents or a diff' .claude/skills/build/SKILL.md",
] as const

// The issue's sketch at 62 columns, 4 minutes in; only the rows tests read.
const SKETCH = [
  'Build #52                                       Working  0h 4m',
  'Keep files, diffs and logs out of the main session of `/build`',
  'Checks pass    ━━━━━━━━━━━━━─────────────────────────   1 of 3',
  'Pauses in      ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━  23h 56m',
  'Stops refused  ━━━───────────────────────────────────  2 of 30',
] as const

const phases = ['explore', 'test', 'implement', 'review', 'pr', 'ci', 'retro'] as const
const NAMES = ['Explore', 'Test', 'Implement', 'Review', 'Pull request', 'CI', 'Retro']
const STATE = (phase: string, status = 'working') =>
  `status: ${status}\nissue: 12 Reject empty files\nphase: ${phase}\ncheck: a\ncheck: b\n`

test('no build shows how to start one', async ($, on) => {
  world(on)
  await $.session.start(START)

  for (const surface of SURFACES) {
    const ui = await $.ui.mount({ ...PANE, surface })

    expect((await ui.find({ type: 'Text', text: /No build/ }))?.text).toBe(
      'No build is running. Start one with /build and an issue number.',
    )
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

  const rows = await paneRows($, 'terminal')

  expect(rows[0]).toBe(header('Build', 'Needs you', 40))
  expect(rows.filter(one => one.startsWith('Pauses in'))).toEqual([])
})

test('a running build is polled; a finished one is marked done and no longer polled', async ($, on) => {
  const state = 'status: working\nissue: 12 Reject empty files\nphase: pr\ncheck: true\npr: https://example.test/pull/1\n'
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

  const rows = await paneRows($, 'terminal', 71)

  expect(rows).toContain('✔ Explore  ✔ Test  ✔ Implement  ✔ Review  ✔ Pull request  ✔ CI  ✔ Retro')
  expect(rows).toContain('Pull request #1')
  expect(rows.filter(one => one.includes('example.test'))).toEqual([])
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

test('until the build writes its state, the pane shows issue #10 at the first phase', async ($, on) => {
  const files = new Map<string, string>()
  const { clock, act } = world(on, files)
  const { run, call } = act($)
  await $.session.start(START)
  await clock.settle()
  await run('build', '10')
  await call()
  await clock.advance(60_000)

  expect(await headEach($)).toEqual(every(header('Build #10', 'Working  0h 1m', 40)))
  expect(await currentEach($)).toEqual(every(['◉ Explore']))

  files.set('state', 'status: working\nissue: 10 Some title\nphase: test\n')
  await call()

  expect(await headEach($)).toEqual(every(header('Build #10', 'Working  0h 1m', 40)))
  expect((await rowsEach($)).map(rows => rows[1])).toEqual(every('Some title'))
  expect(await currentEach($)).toEqual(every(['◉ Test']))
})

test('/build 18 then entering a worktree still shows issue #18', async ($, on) => {
  const finished = 'status: working\nissue: 13 Some title\nphase: pr\ncheck: true\npr: https://example.test/pull/13\n'
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

  expect(await headEach($)).toEqual(every(header('Build #18', 'Working  0h 0m', 40)))
  expect(await currentEach($)).toEqual(every(['◉ Explore']))
})

test('/build whose arguments do not start with a number shows a heading without a number', async ($, on) => {
  const { clock, act, isOpen } = world(on)
  const { run, close } = act($)
  await $.session.start(START)
  await clock.settle()

  for (const args of ['', 'abc', 'abc 18', '#18']) {
    await run('build', args)

    expect(isOpen()).toBe(true)
    expect(await headEach($)).toEqual(every(header('Build', 'Working  0h 0m', 40)))
    await close()
  }
})

test('/build 10 shows issue #10, not a finished build still on disk', async ($, on) => {
  const old = 'status: working\nissue: 12 Reject empty files\nphase: pr\ncheck: true\npr: https://example.test/pull/1\n'
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

  expect(await headEach($)).toEqual(every(header('Build #10', 'Working  0h 0m', 40)))
  expect(await currentEach($)).toEqual(every(['◉ Explore']))
  expect((await rowsEach($)).map(rows => rows.filter(one => one.startsWith('Pull request #')))).toEqual(every([]))
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
    files.set('state', `status: working\nissue: ${issue} Some title\nphase: pr\npr: https://example.test/pull/${issue}\n`)
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

test("where no pane can be drawn, /build prints the pane's words before its first tool call", async ($, on) => {
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
  expect(logged).toContain('Build #10')
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

test('a build in a worktree the session entered shows its own issue, phase and checks', async ($, on) => {
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

  expect(await headEach($)).toEqual(every(header('Build #12', 'Working', 40)))
  expect((await rowsEach($)).map(rows => rows[1])).toEqual(every('Reject empty files'))
  expect(await currentEach($)).toEqual(every(['◉ Test']))

  for (const rows of await rowsEach($)) {
    expect(rows).toContain('✘ false')
  }

  await $.tool.call({ tool: 'EnterWorktree', path: '/wt' })
  await clock.settle()

  expect(await headEach($)).toEqual(every(header('Build #19', 'Working', 40)))
  expect((await rowsEach($)).map(rows => rows[1])).toEqual(every('Show every build'))
  expect(await currentEach($)).toEqual(every(['◉ Implement']))

  for (const rows of await rowsEach($)) {
    expect(rows).toContain('✔ sh a')
    expect(rows).toContain('○ sh b')
    expect(rows.filter(one => one.startsWith('✘ '))).toEqual([])
  }
})

test("the pane lists the build skill's phases and marks the current one", async ($, on) => {
  const files = new Map<string, string>()
  const { clock, act } = world(on, files)
  const { call } = act($)
  await $.session.start(START)
  await clock.settle()
  const row = (mark: (at: number) => string) => NAMES.map((name, at) => `${mark(at)} ${name}`).join('  ')
  const phaseRows = (rows: string[]) => rows.filter(one => /^[✔◉○] Explore/.test(one))

  for (const [index, phase] of phases.entries()) {
    files.set('state', STATE(phase))
    await call()

    for (const rows of await rowsEach($, 71)) {
      expect(phaseRows(rows)).toEqual([row(at => (at < index ? '✔' : at === index ? '◉' : '○'))])
    }
  }

  files.set('state', STATE('pr', 'blocked'))
  await call()

  expect(phaseRows(await paneRows($, 'terminal', 71))).toEqual([row(at => (at < 4 ? '✔' : at === 4 ? '◉' : '○'))])

  files.set('state', STATE('pr'))
  files.set('done', STATE('pr'))
  await call()

  expect(phaseRows(await paneRows($, 'terminal', 71))).toEqual([row(() => '✔')])
})

test('the status line says what the build is doing in one sentence', async ($, on) => {
  const files = new Map<string, string>()
  const { clock, act } = world(on, files)
  const { call } = act($)
  const cases = [
    [STATE('explore'), 'Mapping the code and the spec.'],
    [STATE('test'), 'Writing tests that fail until the issue is done.'],
    [STATE('implement'), 'Changing the code until every check passes.'],
    [STATE('review'), 'Reviewing the diff against the issue.'],
    [STATE('pr'), 'Opening the pull request.'],
    [STATE('ci'), 'Waiting for CI to pass.'],
    [STATE('retro'), 'Posting ideas for future issues and a retro.'],
    [STATE('other'), 'Starting the build.'],
    [STATE('implement', 'blocked'), 'Waiting for your answer.'],
    [STATE('pr'), 'CI passes; the pull request is ready to merge.'],
  ] as const
  await $.session.start(START)
  await clock.settle()

  for (const [state, sentence] of cases) {
    files.set('state', state)

    if (sentence === 'CI passes; the pull request is ready to merge.') {
      files.set('done', state)
    }
    await call()

    for (const rows of await rowsEach($, 80)) {
      expect(rows.filter(one => one === sentence)).toEqual([sentence])
    }
  }
})

test('the pane is titled Build #<issue>', async ($, on) => {
  const files = new Map([
    ['state', 'status: working\nissue: 12 Reject empty files\nphase: test\ncheck: true\ncheck: false\n'],
    ['results', 'pass true\nfail false\n'],
  ])
  const { clock, seen, act } = world(on, files)
  const { run } = act($)
  await $.session.start(START)
  await clock.settle()

  expect((await run('voyage', '')).text).toBe('Build pane opened.')
  expect(seen.titles.at(-1)).toBe('Build #12')

  await run('build', '7')

  expect(seen.titles.at(-1)).toBe('Build #7')

  files.delete('state')
  await run('build', 'abc')

  expect(seen.titles.at(-1)).toBe('Build')
})

test('no starship word appears in the pane, the toast or the text answer', async ($, on) => {
  const files = new Map<string, string>()
  const { clock, seen, act } = world(on, files)
  const { run, call } = act($)
  const words =
    /\b(voyages?|stars?|constellations?|fuel|shields?|dock(ed|ing)?|drones?|orbits?|nebulae?|hulls?|crew|mission control)\b/i
  const texts: string[] = []
  const collect = async () => {
    for (const surface of SURFACES) {
      texts.push(...(await lines($, surface)))
    }

    for (const isPlaced of [true, false]) {
      seen.isPlaced = isPlaced
      texts.push((await run('voyage', '')).text ?? '')
    }
  }
  await $.session.start(START)
  await clock.settle()
  await collect()

  for (const phase of [...phases, 'other']) {
    files.set('state', `${STATE(phase)}pr: https://example.test/1\n`)
    files.set('results', phase === 'review' ? 'pass a\npass b\n' : phase === 'implement' ? 'pass a\nfail b\n' : '')
    files.set('started', '96400\n')
    files.set('refusals', '2\n')
    await call()
    await collect()
  }

  files.set('state', STATE('implement', 'blocked'))
  await call()
  await collect()
  files.set('state', `${STATE('pr')}pr: https://example.test/1\n`)
  files.set('done', files.get('state') ?? '')
  await call()
  await collect()

  const all = [...texts, ...seen.toasts, ...seen.titles]

  expect(seen.toasts).not.toEqual([])
  expect(all.filter(one => words.test(one))).toEqual([])
})

const usage = (input: number, output: number, read: number, creation: number): TurnUsage => ({
  model: 'claude-haiku-5-5',
  input_tokens: input,
  output_tokens: output,
  cache_read_input_tokens: read,
  cache_creation_input_tokens: creation,
})

// An agent of the build, started by `parentAgentId` (the main loop when absent).
const spawn = async ($: Engine, description: string, parentAgentId?: string, model?: string): Promise<string> => {
  const { agentId } = await $.agent.spawn({
    tool_use_id: description,
    prompt: '',
    description,
    subagentType: 'general-purpose',
    provider: { plugin: 'engine', tier: 'core' },
    parentModel: 'claude-opus-5',
    background: false,
    fork: false,
    ...(parentAgentId === undefined ? {} : { parentAgentId }),
    ...(model === undefined ? {} : { model }),
  })

  if (agentId === undefined) {
    throw new Error('no agent started')
  }

  return agentId
}

// A turn of `agentId` (the main loop when absent) ends having cost `cost`.
const complete = ($: Engine, cost: TurnUsage, agentId?: string) =>
  $.turn.complete({
    answer: '',
    durationMs: 1000,
    isAborted: false,
    turnId: agentId ?? 'main',
    reason: 'answer',
    usage: cost,
    ...(agentId === undefined ? {} : { agentId }),
  })

const agentRows = (rows: string[]): string[] => tableOf(rows).slice(1, -1)

test('an agent whose turn starts again is running again, keeping its model and tokens', async ($, on) => {
  const { clock, act } = world(on)
  await $.session.start(START)
  await clock.settle()
  await act($).run('build', '10')
  const tests = await spawn($, 'Write the tests', undefined, 'claude-sonnet-5-5')
  await complete($, usage(1, 2, 3, 4), tests)
  await $.turn.start({ text: '', turnId: tests, agentId: tests })
  await clock.settle()

  for (const rows of await rowsEach($)) {
    expect(agentRows(rows)).toEqual([
      '◉ Main session     fable-5-2        –',
      '◉ Write the tests  sonnet-5-5      10',
    ])
  }
})

test('a title too long for the pane is cut with an ellipsis, never its model or tokens', async ($, on) => {
  const { clock, act } = world(on)
  await $.session.start(START)
  await clock.settle()
  await act($).run('build', '10')
  const review = await spawn($, 'Review the whole renderer carefully')
  await complete($, usage(100, 200, 300, 634), review)
  await clock.settle()

  for (const rows of await rowsEach($)) {
    expect(tableOf(rows)).toEqual([
      '  Agent                Model      Tokens',
      '◉ Main session         fable-5-2       –',
      '✔ Review the whole r…  haiku-5-5   1,234',
      '  Total                            1,234',
    ])
  }
})

test('a Total row under the agents sums every token, the main session included', async ($, on) => {
  const { clock, act } = world(on)
  await $.session.start(START)
  await clock.settle()
  await act($).run('build', '10')
  const parser = await spawn($, 'Map the parser')
  const lexer = await spawn($, 'Read the lexer', parser)
  await complete($, usage(1, 2, 4, 8))
  await complete($, usage(10, 20, 30, 40), parser)
  await complete($, usage(1, 1, 1, 0), lexer)
  await clock.settle()

  for (const rows of await rowsEach($)) {
    expect(tableOf(rows)).toEqual([
      '  Agent             Model      Tokens',
      '◉ Main session      fable-5-2      15',
      '✔ Map the parser    haiku-5-5     100',
      '✔   Read the lexer  haiku-5-5       3',
      '  Total                           118',
    ])
  }
})

test('the pane lists every agent the build spawned as a tree', async ($, on) => {
  const { clock, act } = world(on)
  await $.session.start(START)
  await clock.settle()
  await act($).run('build', '10')
  const parser = await spawn($, 'Map the parser')
  const lexer = await spawn($, 'Read the lexer', parser)
  await spawn($, 'Write the docs')
  await spawn($, 'Check tokens', lexer)
  await clock.settle()

  for (const rows of await rowsEach($)) {
    expect(agentRows(rows)).toEqual([
      '◉ Main session      fable-5-2       –',
      '◉ Map the parser    haiku-5-5       –',
      '◉   Read the lexer  haiku-5-5       –',
      '◉     Check tokens  haiku-5-5       –',
      '◉ Write the docs    haiku-5-5       –',
    ])
  }
})

test('the elapsed time ticks while the build runs and freezes when it is done', async ($, on) => {
  const files = new Map<string, string>()
  const { clock, act } = world(on, files)
  await $.session.start(START)
  await clock.settle()
  await act($).run('build', '10')

  expect(await headEach($)).toEqual(every(header('Build #10', 'Working  0h 0m', 40)))

  await clock.advance(125_000)

  expect(await headEach($)).toEqual(every(header('Build #10', 'Working  0h 2m', 40)))

  const state = 'status: working\nissue: 10 Some title\nphase: pr\n'
  files.set('state', state)
  files.set('done', state)
  await clock.advance(5000)
  await clock.advance(3_600_000)

  expect(await headEach($)).toEqual(every(header('Build #10', 'Done  0h 2m', 40)))
})

const sketch = async ($: Engine, on: On) => {
  const files = new Map<string, string>()
  const { clock, act } = world(on, files)
  const { call } = act($)
  await $.session.start(START)
  await clock.settle()
  files.set(
    'state',
    `status: working\nissue: 52 ${TITLE}\nphase: implement\n${CHECKS.map(one => `check: ${one}\n`).join('')}`,
  )
  files.set('results', `pass ${CHECKS[0]}\nfail ${CHECKS[1]}\nfail ${CHECKS[2]}\n`)
  files.set('started', '100000\n')
  files.set('refusals', '2\n')
  await call()
  await clock.advance(240_000)
}

test('the first row reads Build #<number> and ends with the state and elapsed time', async ($, on) => {
  const files = new Map([['state', `status: working\nissue: 52 ${TITLE}\nphase: test\n`]])
  const { clock, act } = world(on, files)
  const { run, call } = act($)
  await $.session.start(START)
  await clock.settle()

  // No /build ran in this session: no elapsed time.
  for (const rows of await rowsEach($, 62)) {
    expect(rows.slice(0, 2)).toEqual([header('Build #52', 'Working', 62), TITLE])
  }

  await run('build', '52')
  files.set('state', `status: working\nissue: 52 ${TITLE}\nphase: implement\n`)
  await call()
  await clock.advance(240_000)

  for (const rows of await rowsEach($, 62)) {
    expect(rows.slice(0, 2)).toEqual([SKETCH[0], SKETCH[1]])
  }

  expect(await coloursOf($, /Working/)).toEqual(every('suggestion'))

  files.set('state', `status: blocked\nissue: 52 ${TITLE}\nphase: implement\n`)
  await call()

  expect((await rowsEach($, 62)).map(rows => rows[0])).toEqual(every(header('Build #52', 'Needs you  0h 4m', 62)))
  expect(await coloursOf($, /Needs you/)).toEqual(every('warning'))

  files.set('state', `status: working\nissue: 52 ${TITLE}\nphase: pr\n`)
  files.set('done', `status: working\nissue: 52 ${TITLE}\nphase: pr\n`)
  await call()

  expect((await rowsEach($, 62)).map(rows => rows[0])).toEqual(every(header('Build #52', 'Done  0h 4m', 62)))
  expect(await coloursOf($, /Done/)).toEqual(every('success'))
})

test('the phases share one row and wrap only when the pane is too narrow', async ($, on) => {
  const files = new Map([['state', 'status: working\nissue: 12 Reject empty files\nphase: implement\n']])
  const { clock } = world(on, files)
  await $.session.start(START)
  await clock.settle()

  const cases = [
    [71, ['✔ Explore  ✔ Test  ◉ Implement  ○ Review  ○ Pull request  ○ CI  ○ Retro']],
    [62, ['✔ Explore  ✔ Test  ◉ Implement  ○ Review  ○ Pull request  ○ CI', '○ Retro']],
    [40, ['✔ Explore  ✔ Test  ◉ Implement  ○ Review', '○ Pull request  ○ CI  ○ Retro']],
    [39, ['✔ Explore  ✔ Test  ◉ Implement', '○ Review  ○ Pull request  ○ CI  ○ Retro']],
  ] as const

  for (const [columns, expected] of cases) {
    for (const rows of await rowsEach($, columns)) {
      expect(rows.slice(2, 2 + expected.length)).toEqual(expected)
    }
  }
})

test('phases, checks and agents share one set of marks and no emoji', async ($, on) => {
  const files = new Map<string, string>()
  const { clock, act } = world(on, files)
  const { run, call } = act($)
  await $.session.start(START)
  await clock.settle()
  await run('build', '12')
  files.set('state', `${STATE('review')}check: c\n`)
  files.set('results', 'pass a\nfail b\n')
  await call()
  const parser = await spawn($, 'Map the parser')
  await spawn($, 'Write the tests')
  await complete($, usage(100, 200, 300, 634), parser)
  await clock.settle()

  for (const rows of await rowsEach($, 71)) {
    const marked = rows.filter(one => /^[✔◉✘○] /.test(one))

    expect(marked.map(one => one.replace(/ {2,}.*/, ''))).toEqual([
      '✔ Explore',
      '✔ a',
      '✘ b',
      '○ c',
      '◉ Main session',
      '✔ Map the parser',
      '◉ Write the tests',
    ])
    expect(rows.filter(one => /\p{Emoji_Presentation}|[✓✗◎·]/u.test(one))).toEqual([])
  }
})

test('checks, time left and refused stops are aligned rows of label, bar and value', async ($, on) => {
  await sketch($, on)
  const gauges = (rows: string[]) => rows.filter(one => /^(Checks pass|Pauses in|Stops refused) /.test(one))

  for (const rows of await rowsEach($, 62)) {
    expect(gauges(rows)).toEqual(SKETCH.slice(2))
  }

  // At 40 columns labels, bars and values still line up.
  for (const rows of await rowsEach($, 40)) {
    const parts = gauges(rows).map(one => /^(.{15})([━─]{16})  (.{7})$/.exec(one))

    expect(parts.map(one => one?.[1])).toEqual(['Checks pass    ', 'Pauses in      ', 'Stops refused  '])
    expect(parts.map(one => one?.[3])).toEqual([' 1 of 3', '23h 56m', '2 of 30'])
    expect(gauges(rows).map(one => one.length)).toEqual([40, 40, 40])
  }
})

test('before a check is defined the checks row reads none yet', async ($, on) => {
  const files = new Map([['state', 'status: working\nissue: 12 Reject empty files\nphase: explore\n']])
  const { clock } = world(on, files)
  await $.session.start(START)
  await clock.settle()

  for (const rows of await rowsEach($, 62)) {
    expect(rows.filter(one => one.startsWith('Checks pass'))).toEqual([`Checks pass    ${'─'.repeat(37)}  none yet`])
    expect(rows.filter(one => one.includes('0 of 0'))).toEqual([])
  }
})

test('a done build draws no Pauses in or Stops refused row', async ($, on) => {
  const state = 'status: working\nissue: 12 Reject empty files\nphase: pr\ncheck: a\ncheck: b\n'
  const files = new Map([
    ['state', state],
    ['results', 'pass a\npass b\n'],
    ['started', '96400\n'],
    ['refusals', '3\n'],
  ])
  const { clock } = world(on, files)
  await $.session.start(START)
  await clock.settle()
  const names = (rows: string[]) => rows.filter(one => /^(Pauses in|Stops refused) /.test(one)).map(one => one.slice(0, 13).trim())

  for (const rows of await rowsEach($, 62)) {
    expect(names(rows)).toEqual(['Pauses in', 'Stops refused'])
  }

  files.set('done', state)
  await clock.advance(5000)

  for (const rows of await rowsEach($, 62)) {
    expect(names(rows)).toEqual([])
    expect(rows.filter(one => one.startsWith('Checks pass'))).toEqual([`Checks pass    ${'━'.repeat(39)}  2 of 2`])
  }
})

test('a recorded pull request is a link under the status sentence', async ($, on) => {
  const files = new Map([['state', 'status: working\nissue: 52 Reject empty files\nphase: ci\ncheck: a\n']])
  const { clock } = world(on, files)
  await $.session.start(START)
  await clock.settle()

  expect(await linksEach($)).toEqual(every([]))

  files.set('state', 'status: working\nissue: 52 Reject empty files\nphase: ci\ncheck: a\npr: https://example.test/pull/52\n')
  await clock.advance(5000)

  expect(await linksEach($)).toEqual(every([{ href: 'https://example.test/pull/52', text: 'Pull request #52' }]))

  for (const rows of await rowsEach($, 62)) {
    const at = rows.indexOf('Waiting for CI to pass.')

    expect(at).toBeGreaterThan(-1)
    expect(rows[at + 1]).toBe('Pull request #52')
    expect(rows.filter(one => one.includes('example.test'))).toEqual([])
  }

  for (const surface of SURFACES) {
    expect((await lines($, surface)).filter(one => one.includes('example.test'))).toEqual([])
  }
})

// A running build: Main session, one done agent, one not counted yet.
const scene = async ($: Engine, on: On) => {
  const files = new Map<string, string>()
  const { clock, act } = world(on, files)
  await $.session.start(START)
  await clock.settle()
  await act($).run('build', '10')
  const parser = await spawn($, 'Map the parser')
  await spawn($, 'Write the tests')
  await complete($, usage(1, 2, 3, 4))
  await complete($, usage(100, 200, 300, 634), parser)
  await clock.settle()

  return { files, clock, act }
}

const AGENT = /^[◉✔✘○] +(Main session|Map the parser|Write the tests)/

test('the agent table has a header row', async ($, on) => {
  await scene($, on)

  for (const rows of await rowsEach($, 62)) {
    expect(rows.indexOf('  Agent            Model      Tokens')).toBe(rows.findIndex(one => AGENT.test(one)) - 1)
  }
})

test('an agent with no tokens counted shows a dash', async ($, on) => {
  await scene($, on)

  for (const rows of await rowsEach($, 62)) {
    expect(tableOf(rows).filter(one => /–$/.test(one))).toEqual(['◉ Write the tests  haiku-5-5       –'])
    expect(tableOf(rows).filter(one => /(^|\s)0$/.test(one))).toEqual([])
  }
})

test('Main session is the first row of the agent table', async ($, on) => {
  const { files, act } = await scene($, on)

  for (const rows of await rowsEach($, 62)) {
    expect(rows.filter(one => AGENT.test(one))).toEqual([
      '◉ Main session     fable-5-2      10',
      '✔ Map the parser   haiku-5-5   1,234',
      '◉ Write the tests  haiku-5-5       –',
    ])
    expect(tableOf(rows).at(-1)).toBe('  Total                        1,244')
  }

  const state = 'status: working\nissue: 10 Some title\nphase: pr\n'
  files.set('state', state)
  files.set('done', state)
  await act($).call()

  for (const rows of await rowsEach($, 62)) {
    expect(rows.filter(one => AGENT.test(one))[0]).toBe('✔ Main session     fable-5-2      10')
  }
})

test('before the first agent is spawned no table is drawn', async ($, on) => {
  const { clock, act } = world(on)
  await $.session.start(START)
  await clock.settle()
  await act($).run('build', '10')

  for (const rows of await rowsEach($, 62)) {
    expect(rows.filter(one => /Main session|^ {2}(Agent|Total)( |$)/.test(one))).toEqual([])
  }

  await spawn($, 'Map the parser')

  for (const rows of await rowsEach($, 62)) {
    expect(tableOf(rows)).not.toEqual([])
  }
})

test('in a pane 30 columns wide only check commands and agent titles are cut', async ($, on) => {
  const check = 'grep -q needle docs/guide/long-name.md'
  const files = new Map<string, string>()
  const { clock, act } = world(on, files, true, () => '/repo', 'claude-opus-5')
  await $.session.start(START)
  await clock.settle()
  await act($).run('build', '52')
  files.set(
    'state',
    `status: working\nissue: 52 ${TITLE} docs/guide/a-long-file-name-here.md\nphase: implement\npr: https://example.test/pull/52\ncheck: ${check}\n`,
  )
  files.set('results', `pass ${check}\n`)
  files.set('started', '96400\n')
  files.set('refusals', '3\n')
  await act($).call()
  const review = await spawn($, 'Reviewing renderer carefully', undefined, 'claude-opus-5')
  await complete($, usage(100, 200, 300, 634), review)
  await clock.settle()

  for (const rows of await rowsEach($, 30)) {
    expect(rows.filter(one => one.length > 30)).toEqual([])
    expect(rows.filter(one => one.includes('…'))).toEqual(['✔ grep -q needle docs/guide/l…', '✔ Reviewing r…  opus-5   1,234'])
    expect(rows.slice(0, 6)).toEqual([
      header('Build #52', 'Working  0h 0m', 30),
      'Keep files, diffs and logs out',
      'of the main session of',
      '`/build`',
      'docs/guide/a-long-file-name-he',
      're.md',
    ])
    expect(rows.slice(9, 12)).toEqual([
      'Changing the code until every',
      'check passes.',
      'Pull request #52',
    ])
    expect(rows.slice(-4)).toEqual([
      '  Agent         Model   Tokens',
      '◉ Main session  opus-5       –',
      '✔ Reviewing r…  opus-5   1,234',
      '  Total                  1,234',
    ])
  }
})

test('the /voyage text answer gives the same facts in the same words without bars', async ($, on) => {
  const files = new Map<string, string>()
  const { clock, act } = world(on, files, false)
  const { run, call } = act($)
  await $.session.start(START)
  await clock.settle()
  await run('build', '12')
  files.set(
    'state',
    'status: working\nissue: 12 Reject empty files\nphase: test\ncheck: true\ncheck: false\npr: https://example.test/pull/12\n',
  )
  files.set('results', 'pass true\nfail false\n')
  files.set('started', '96525\n')
  files.set('refusals', '2\n')
  await call()
  const parser = await spawn($, 'Map the parser and every file it reads')
  await complete($, usage(1, 2, 3, 4))
  await complete($, usage(10, 20, 30, 40), parser)
  await clock.advance(125_000)
  const answer = ((await run('voyage', '')).text ?? '').split('\n')
  const facts = [
    'Build #12  Working  0h 2m',
    'Reject empty files',
    '✔ Explore  ◉ Test  ○ Implement  ○ Review  ○ Pull request  ○ CI  ○ Retro',
    'Writing tests that fail until the issue is done.',
    'Pull request #12 https://example.test/pull/12',
    'Checks pass    1 of 2',
    'Pauses in      23h 0m',
    'Stops refused  2 of 30',
    '✔ true',
    '✘ false',
    '  Agent                                   Model      Tokens',
    '◉ Main session                            fable-5-2      10',
    '✔ Map the parser and every file it reads  haiku-5-5     100',
    '  Total                                                 110',
  ]

  expect(answer.slice(-2)).toEqual(['', `No pane here: ${REASON}`])
  expect(answer.slice(0, -2).filter(one => one.trim() !== '')).toEqual(facts)
  expect(answer.join('\n')).not.toMatch(/[━─]/)
  expect(answer.join('\n')).not.toContain('pane opened')

  // The pane says the same, but for its bars and its right-aligned header and values.
  const same = (one: string) =>
    one
      .replace(/^(Build #\d+) {2,}(Working|Needs you|Done)/, '$1  $2')
      .replace(/^(Checks pass|Pauses in|Stops refused) +[━─]+ +/, (_, label: string) => label.padEnd(15))
      .replace(/ https:\/\/\S+$/, '')

  expect((await paneRows($, 'terminal', 100)).map(same)).toEqual(facts.map(same))
})
