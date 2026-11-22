import assert from 'node:assert/strict'
import { mkdtempSync } from 'node:fs'
import test from 'node:test'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { dispatchEnvelope } from './dispatch.mjs'

test('openrig handoff uses queue create and keeps the inbox when rig fails', () => {
  const dir = mkdtempSync(join(tmpdir(), 'dendro-dispatch-'))
  const calls = []
  const result = dispatchEnvelope(dir, {
    to: 'rev.review',
    from: 'dev.build',
    outcome: 'no_change',
    queueAction: 'handoff',
    summary: 'contract',
    openrigActive: true,
    rigRunner: (args) => {
      calls.push(args)
      return { status: 1, stderr: 'unknown flag sk-secretvalue\n' }
    },
  })
  assert.equal(result.mode, 'inbox-fallback')
  assert.equal(result.rigFailed, true)
  assert.equal(result.openrigCli, 'downgraded-handoff')
  assert.deepEqual(calls[0], ['queue', 'create', '--destination', 'rev.review', '--summary', 'contract'])
  assert.equal(result.rigStderr.includes('sk-secretvalue'), false)
  assert.match(result.rigStderr, /\[redacted\]/)
})
