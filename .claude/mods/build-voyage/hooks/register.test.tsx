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
const world = (on: On, files: Record<string, string>) => {
  const clock = mock.clock(on, { now: 100_000_000 })
  on('command.register', () => ({ value: { command: 'voyage' } }))
  on('ui.open', () => ({ value: { isPlaced: true } }))
  on('ui.toast', () => ({ value: undefined }))
  on('session.start', (_, e) => ({ cwd: e.cwd }))
  on('fs.read', (_, e) => {
    const name = Object.keys(files).find(one => e.path.endsWith(one))

    if (name === undefined) {
      throw new Error('ENOENT')
    }

    return { value: files[name] ?? '' }
  })

  return clock
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
  world(on, {
    // Started one hour before now: 23 of the 24 hours are left.
    '.claude/build/state':
      'status: working\nissue: 12 Reject empty files\nphase: implement\n' +
      'check: true\ncheck: false\ncheck: sh x\nstarted: 96400\n',
    '.claude/build/results': 'pass true\nfail false\n',
    '.claude/build/refusals': '3\n',
  })
  await $.session.start(START)

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

test('a blocked build holds orbit, and a new file is picked up by the timer', async ($, on) => {
  const files: Record<string, string> = {}
  const clock = world(on, files)
  await $.session.start(START)
  files['.claude/build/state'] = 'status: blocked\ncheck: true\n'
  await clock.advance(5000)

  const ui = await $.ui.mount({ ...PANE, surface: 'terminal' })

  expect(await ui.find({ type: 'Text', text: /Holding orbit/ })).toBeDefined()
  expect(await ui.find({ type: 'Text', text: /uncharted/ })).toBeDefined()
  expect(await bar(ui, /Fuel/)).toBeUndefined()
  await ui.unmount()
})
