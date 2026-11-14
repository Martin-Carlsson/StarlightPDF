import { expect, mock, test } from 'claude-code/testing'
import type { ElementQuery, FoundElement } from 'claude-code/testing'
import type { On } from 'claude-code'

const PANE = {
  plugin: 'build-voyage',
  component: 'Pane',
  requestId: 'build-voyage',
  props: {
    title: 'Starlight voyage',
    isFocused: false,
    bodyColumns: 40,
    placement: 'dock',
    scroll: { offset: 0, bodyRows: 40 },
    view: {},
  },
} as const

// Stands for the engine: the files of .claude/build/, the clock at 100 000 s.
const world = (on: On, files: Record<string, string>, isPlaced: boolean = true) => {
  const clock = mock.clock(on, { now: 100_000_000 })
  const reads = { count: 0 }
  on('command.register', () => ({ value: { command: 'voyage' } }))
  on('ui.open', () => ({ value: isPlaced ? { isPlaced } : { isPlaced, reason: 'no surface places panes' } }))
  on('ui.toast', () => ({ value: undefined }))
  on('session.start', (_, e) => ({ cwd: e.cwd }))
  on('tool.call', () => ({ result: {} }))
  on('fs.read', (_, e) => {
    reads.count += 1
    const name = Object.keys(files).find(one => e.path.endsWith(one))

    if (name === undefined) {
      throw new Error('ENOENT')
    }

    return { value: files[name] ?? '' }
  })

  return { clock, reads }
}

// A Progress is drawn as a Text and a bar on some surfaces, natively on others.
type Drawing = { find: (query: ElementQuery) => Promise<FoundElement | undefined> }

const bar = async (ui: Drawing, word: RegExp): Promise<string | undefined> => {
  const drawn = await ui.find({ type: 'Text', text: word })
  const native = await ui.find({ type: 'Progress', text: word })

  return drawn?.text ?? (native && `${String(native.props.label)} ${String(native.props.valueText)}`)
}

const START = { cwd: '/', surface: 'desktop', isInteractive: true } as const
const SURFACES = ['terminal', 'desktop', 'vscode', 'mobile'] as const

test('a build shows its checks, stage and fuel on every surface', async ($, on) => {
  const { clock } = world(on, {
    // Started one hour before now: 23 of the 24 hours are left.
    '.claude/build/state':
      'status: working\nissue: 12 Reject empty files\nphase: implement\n' +
      'check: true\ncheck: false\ncheck: sh x\n',
    '.claude/build/started': '96400\n',
    '.claude/build/results': 'pass true\nfail false\ntree 4b825dc\n',
    '.claude/build/refusals': '3\n',
  })
  await $.session.start(START)
  await clock.settle()

  for (const surface of SURFACES) {
    const ui = await $.ui.mount({ ...PANE, surface })

    expect((await ui.find({ type: 'Text', text: /issue #/ }))?.text).toBe('Destination issue #12 Reject empty files')
    expect((await ui.find({ type: 'Text', text: /^★/ }))?.text).toBe('★ true')
    expect((await ui.find({ type: 'Text', text: /^☆/ }))?.text).toBe('☆ false')
    expect((await ui.find({ type: 'Text', text: /^·/ }))?.text).toBe('· sh x')
    expect((await ui.find({ type: 'Text', text: /Burn/ }))?.text).toBe('◎ Burn  ⟵ you are here')
    expect((await ui.find({ type: 'Text', text: /Chart/ }))?.text).toBe('◉ Chart')
    expect((await ui.find({ type: 'Text', text: /Dock$/ }))?.text).toBe('○ Dock')
    expect(await bar(ui, /Fuel/)).toBe('Fuel 23h 0m until check-in')
    expect(await bar(ui, /Constellation/)).toBe('Constellation 1 of 3 stars lit')
    expect(await bar(ui, /Shields/)).toBe('Shields 3 of 30 bumps taken')
    await ui.unmount()
  }
})

test('no build shows the dry dock', async ($, on) => {
  world(on, {})
  await $.session.start(START)

  for (const surface of SURFACES) {
    const ui = await $.ui.mount({ ...PANE, surface })

    expect(await ui.find({ type: 'Text', text: /dry dock/ })).toBeDefined()
    await ui.unmount()
  }
})

test('an idle session does not poll; a tool call notices a build', async ($, on) => {
  const files: Record<string, string> = {}
  const { clock, reads } = world(on, files)
  await $.session.start(START)
  await clock.settle()
  const atStart = reads.count
  await clock.advance(60_000)

  expect(reads.count).toBe(atStart)

  files['.claude/build/state'] = 'status: blocked\ncheck: true\n'
  await $.tool.call({ tool: 'Bash', command: 'ls' })
  await clock.settle()

  const ui = await $.ui.mount({ ...PANE, surface: 'terminal' })

  expect(await ui.find({ type: 'Text', text: /Holding orbit/ })).toBeDefined()
  expect(await ui.find({ type: 'Text', text: /uncharted/ })).toBeDefined()
  expect(await bar(ui, /Fuel/)).toBeUndefined()
  await ui.unmount()
})

test('where no pane can be shown, the command answers in text', async ($, on) => {
  const { clock } = world(
    on,
    {
      '.claude/build/state': 'status: working\nissue: 12 Reject empty files\nphase: test\ncheck: true\ncheck: false\n',
      '.claude/build/results': 'pass true\nfail false\n',
    },
    false,
  )
  await $.session.start(START)
  await clock.settle()

  const { text } = await $.command.run({
    command: 'voyage',
    args: '',
    origin: { kind: 'composer' },
    presentation: { isFullscreen: false, columns: 80 },
  })

  expect(text).toContain('issue #12 Reject empty files')
  expect(text).toContain('Chart')
  expect(text).toContain('1 of 2 stars lit')
  expect(text).toContain('★ true')
  expect(text).toContain('☆ false')
  expect(text).toContain('no surface places panes')
  expect(text).not.toContain('pane opened')
})

test('a running build is polled; a finished one is docked and no longer polled', async ($, on) => {
  const state = 'status: working\nissue: 12 Reject empty files\nphase: pr\ncheck: true\npr: https://example.test/1\n'
  const files: Record<string, string> = { '.claude/build/state': state, '.claude/build/results': 'pass true\n' }
  const { clock, reads } = world(on, files)
  await $.session.start(START)
  await clock.settle()
  const running = reads.count
  await clock.advance(5000)

  expect(reads.count).toBeGreaterThan(running)

  // The hook copies the state to `done` when the build finishes.
  files['.claude/build/done'] = state
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
