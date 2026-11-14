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

// Stands for the engine: the session's root, files (a name under the root's .claude/build/, or an
// absolute path), the clock at 100 000 s, its open panes, and `events` in the order they came:
// `build` (the skill), `tool:open` or `tool:closed` (a tool call, by pane), `log:<text>`.
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
  // Toasts and pane titles received, and whether panes are placed.
  const seen = { toasts: [] as string[], titles: [] as string[], isPlaced }
  on('ui.toast', (_, e) => {
    seen.toasts.push(e.text)

    return { value: undefined }
  })
  on('ui.open', (_, e) => {
    seen.titles.push(e.title)
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

// The current phase's mark and name on each surface.
const currentPhase = ($: Engine): Promise<string[]> => shown($, /^◎/, /^◎ .+/)

const every = (text: string): string[] => SURFACES.map(() => text)

test('a build shows its issue and checks on every surface', async ($, on) => {
  const { clock } = world(
    on,
    new Map([
      ['state', 'status: working\nissue: 12 Reject empty files\nphase: implement\ncheck: true\ncheck: false\ncheck: sh x\n'],
      ['results', 'pass true\nfail false\ntree 4b825dc\n'],
    ]),
  )
  await $.session.start(START)
  await clock.settle()

  for (const surface of SURFACES) {
    const ui = await $.ui.mount({ ...PANE, surface })

    expect((await ui.find({ type: 'Text', text: /#12/ }))?.text).toBe('Build #12 Reject empty files')
    expect((await ui.find({ type: 'Text', text: /^✓/ }))?.text).toBe('✓ true')
    expect((await ui.find({ type: 'Text', text: /^✗/ }))?.text).toBe('✗ false')
    expect((await ui.find({ type: 'Text', text: /^·/ }))?.text).toBe('· sh x')
    await ui.unmount()
  }
})

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

  const ui = await $.ui.mount({ ...PANE, surface: 'terminal' })

  expect((await ui.find({ type: 'Text', text: /^Build/ }))?.text).toBe('Build')
  expect(await ui.find({ type: 'Text', text: /left before/ })).toBeUndefined()
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

  expect(text).toContain('Build #12 Reject empty files')
  expect(text).toContain('◎ Test')
  expect(text).toContain('1 of 2 checks pass')
  expect(text).toContain('✓ true')
  expect(text).toContain('✗ false')
  expect(text).toContain(REASON)
  expect(text).not.toContain('pane opened')
})

test('a running build is polled; a finished one is marked done and no longer polled', async ($, on) => {
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

  expect((await ui.find({ type: 'Text', text: /Pull request$/ }))?.text).toBe('◉ Pull request')
  expect((await ui.find({ type: 'Text', text: /example\.test/ }))?.text).toBe('https://example.test/1')
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

test('until the build writes its state, the pane shows issue #10 at the first phase', async ($, on) => {
  const files = new Map<string, string>()
  const { clock, act } = world(on, files)
  const { run, call } = act($)
  await $.session.start(START)
  await clock.settle()
  await run('build', '10')
  await call()
  await clock.advance(60_000)

  expect(await shown($, /^Build/)).toEqual(every('Build #10'))
  expect(await currentPhase($)).toEqual(every('◎ Explore'))

  files.set('state', 'status: working\nissue: 10 Some title\nphase: test\n')
  await call()

  expect(await shown($, /^Build/)).toEqual(every('Build #10 Some title'))
  expect(await currentPhase($)).toEqual(every('◎ Test'))
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

  expect(await shown($, /^Build/)).toEqual(every('Build #18'))
  expect(await currentPhase($)).toEqual(every('◎ Explore'))
})

test('/build whose arguments do not start with a number shows a heading without a number', async ($, on) => {
  const { clock, act, isOpen } = world(on)
  const { run, close } = act($)
  await $.session.start(START)
  await clock.settle()

  for (const args of ['', 'abc', 'abc 18', '#18']) {
    await run('build', args)

    expect(isOpen()).toBe(true)
    expect(await shown($, /^Build/)).toEqual(every('Build'))
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

  expect(await shown($, /^Build/)).toEqual(every('Build #10'))
  expect(await currentPhase($)).toEqual(every('◎ Explore'))
  expect(await shown($, /Pull request open/)).toEqual(every(''))
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

  expect(await shown($, /^Build/)).toEqual(every('Build #12 Reject empty files'))
  expect(await currentPhase($)).toEqual(every('◎ Test'))
  expect(await shown($, /^✗/)).toEqual(every('✗ false'))

  await $.tool.call({ tool: 'EnterWorktree', path: '/wt' })
  await clock.settle()

  expect(await shown($, /^Build/)).toEqual(every('Build #19 Show every build'))
  expect(await currentPhase($)).toEqual(every('◎ Implement'))
  expect(await shown($, /^✓/)).toEqual(every('✓ sh a'))
  expect(await shown($, /^·/)).toEqual(every('· sh b'))
  expect(await shown($, /^✗/)).toEqual(every(''))
})

const phases = ['explore', 'test', 'implement', 'review', 'pr'] as const
const NAMES = ['Explore', 'Test', 'Implement', 'Review', 'Pull request']
const STATE = (phase: string, status = 'working') =>
  `status: ${status}\nissue: 12 Reject empty files\nphase: ${phase}\ncheck: a\ncheck: b\n`

// Every Text of the pane, on one surface.
const lines = async ($: Engine, surface: (typeof SURFACES)[number]): Promise<string[]> => {
  const ui = await $.ui.mount({ ...PANE, surface })
  const all = (await ui.findAll({ type: 'Text' })).map(one => one.text)
  await ui.unmount()

  return all
}

test("the pane lists the build skill's phases and marks the current one", async ($, on) => {
  const files = new Map<string, string>()
  const { clock, act } = world(on, files)
  const { call } = act($)
  await $.session.start(START)
  await clock.settle()

  for (const [index, phase] of phases.entries()) {
    files.set('state', STATE(phase))
    await call()

    for (const surface of SURFACES) {
      expect((await lines($, surface)).filter(one => /^[◉◎○] /.test(one))).toEqual(
        NAMES.map((name, at) => `${at < index ? '◉' : at === index ? '◎' : '○'} ${name}`),
      )
    }
  }

  files.set('state', STATE('pr', 'blocked'))
  await call()

  expect((await lines($, 'terminal')).filter(one => one.startsWith('◎ '))).toEqual(['◎ Pull request'])

  files.set('state', STATE('pr'))
  files.set('done', STATE('pr'))
  await call()

  expect((await lines($, 'terminal')).filter(one => /^[◉◎○] /.test(one))).toEqual(NAMES.map(name => `◉ ${name}`))
})

test('each gauge says what it measures', async ($, on) => {
  const { clock } = world(
    on,
    new Map([
      // Started one hour before now: 23 of the 24 hours are left.
      ['state', 'status: working\nissue: 12 Reject empty files\nphase: implement\ncheck: a\ncheck: b\ncheck: c\n'],
      ['started', '96400\n'],
      ['results', 'pass a\n'],
      ['refusals', '3\n'],
    ]),
  )
  await $.session.start(START)
  await clock.settle()

  for (const surface of SURFACES) {
    expect((await lines($, surface)).filter(one => /checks pass|left before|refused/.test(one))).toEqual([
      '1 of 3 checks pass',
      '23h 0m left before the build pauses',
      '3 of 30 stops refused',
    ])
  }
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
    [STATE('other'), 'Starting the build.'],
    [STATE('implement', 'blocked'), 'Waiting for your answer.'],
    [STATE('pr'), 'Pull request open.'],
  ] as const
  await $.session.start(START)
  await clock.settle()

  for (const [state, sentence] of cases) {
    files.set('state', state)

    if (sentence === 'Pull request open.') {
      files.set('done', state)
    }
    await call()

    expect(await shown($, new RegExp(`^${sentence.replace('.', '\\.')}$`))).toEqual(every(sentence))
  }
})

test('the pane is titled Build #<issue> and /voyage answers in the same words', async ($, on) => {
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

  seen.isPlaced = false
  const { text } = await run('voyage', '')
  const answer = text.split('\n')

  expect(answer.slice(0, 8)).toEqual([
    'Build #12 Reject empty files',
    '◉ Explore',
    '◎ Test',
    '○ Implement',
    '○ Review',
    '○ Pull request',
    'Writing tests that fail until the issue is done.',
    expect.stringMatching(/checks pass/),
  ])
  expect(answer.slice(-2)).toEqual(['', `No pane here: ${REASON}`])

  for (const one of (await lines($, 'terminal')).filter(each => !/^[█░]+$/.test(each))) {
    expect(answer).toContain(one)
  }

  seen.isPlaced = true
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
      texts.push((await run('voyage', '')).text)
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

const agentLines = async ($: Engine, surface: (typeof SURFACES)[number]) =>
  (await lines($, surface)).filter(one => one.includes(' · '))

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

  for (const surface of SURFACES) {
    expect(await agentLines($, surface)).toEqual([
      'Map the parser · claude-haiku-5-5 · running · 0 tokens',
      '  Read the lexer · claude-haiku-5-5 · running · 0 tokens',
      '    Check tokens · claude-haiku-5-5 · running · 0 tokens',
      'Write the docs · claude-haiku-5-5 · running · 0 tokens',
    ])
  }
})

test('each agent shows its task, model, status and tokens', async ($, on) => {
  const { clock, act } = world(on)
  await $.session.start(START)
  await clock.settle()
  await act($).run('build', '10')
  const parser = await spawn($, 'Map the parser')
  const tests = await spawn($, 'Write the tests', undefined, 'claude-sonnet-5-5')
  await complete($, usage(10, 20, 30, 40), parser)
  await complete($, usage(1, 2, 3, 4), tests)
  await $.turn.start({ text: '', turnId: tests, agentId: tests })
  await clock.settle()

  for (const surface of SURFACES) {
    expect(await agentLines($, surface)).toEqual([
      'Map the parser · claude-haiku-5-5 · finished · 100 tokens',
      'Write the tests · claude-sonnet-5-5 · running · 10 tokens',
    ])
  }
})

test('the pane shows one token total for the main loop and every agent', async ($, on) => {
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

  expect(await shown($, /tokens in all/)).toEqual(every('118 tokens in all'))
})

test('the elapsed time ticks while the build runs and freezes when it is done', async ($, on) => {
  const files = new Map<string, string>()
  const { clock, act } = world(on, files)
  await $.session.start(START)
  await clock.settle()
  await act($).run('build', '10')

  expect(await shown($, /elapsed/)).toEqual(every('0h 0m elapsed'))

  await clock.advance(125_000)

  expect(await shown($, /elapsed/)).toEqual(every('0h 2m elapsed'))

  const state = 'status: working\nissue: 10 Some title\nphase: pr\n'
  files.set('state', state)
  files.set('done', state)
  await clock.advance(5000)
  await clock.advance(3_600_000)

  expect(await shown($, /elapsed/)).toEqual(every('0h 2m elapsed'))
})

test('an agent that finished before the pane opened shows its final figures', async ($, on) => {
  const { clock, act } = world(on)
  await $.session.start(START)
  await clock.settle()
  await act($).run('build', '10')
  const parser = await spawn($, 'Map the parser')
  await complete($, usage(10, 20, 30, 40), parser)
  await clock.settle()

  expect(await shown($, /Map the parser/)).toEqual(every('Map the parser · claude-haiku-5-5 · finished · 100 tokens'))
})

test('/voyage without a pane gives the same tree, totals and elapsed time', async ($, on) => {
  const { clock, seen, act } = world(on)
  const { run } = act($)
  seen.isPlaced = false
  await $.session.start(START)
  await clock.settle()
  await run('build', '10')
  const parser = await spawn($, 'Map the parser')
  const lexer = await spawn($, 'Read the lexer', parser)
  await complete($, usage(1, 2, 4, 8))
  await complete($, usage(10, 20, 30, 40), parser)
  await complete($, usage(1, 1, 1, 0), lexer)
  await clock.advance(125_000)
  const answer = (await run('voyage', '')).text.split('\n')

  expect(answer).toContain('0h 2m elapsed')
  expect(answer).toContain('118 tokens in all')
  expect(answer.filter(one => one.includes(' · '))).toEqual([
    'Map the parser · claude-haiku-5-5 · finished · 100 tokens',
    '  Read the lexer · claude-haiku-5-5 · finished · 3 tokens',
  ])
})
