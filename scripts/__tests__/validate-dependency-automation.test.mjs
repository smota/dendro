import test from 'node:test'
import assert from 'node:assert/strict'
import {
  checkDependencyAutomation,
  isAllowedDependencyPath,
} from '../validate-dependency-automation.mjs'

const bot = {
  hash: 'a'.repeat(40),
  authorEmail: '49699333+dependabot[bot]@users.noreply.github.com',
  parents: ['b'.repeat(40)],
}
const paths = ['Cargo.lock']

test('allows only dependency manifests, lockfile and workflow files', () => {
  for (const path of ['Cargo.toml', 'Cargo.lock', 'crates/x/Cargo.toml', '.github/workflows/ci.yml']) {
    assert.equal(isAllowedDependencyPath(path), true, path)
  }
  for (const path of [
    'crates/x/src/lib.rs', 'crates/../Cargo.toml', '.github/CODEOWNERS',
    'scripts/validate-path-ownership.mjs', '../Cargo.toml', 'crates/x/y/Cargo.toml',
    '.github/workflows/sub/ci.yml', '.github/workflows/ci.yaml', './Cargo.toml',
    'crates\\x\\Cargo.toml', 'Cargo.toml\n',
  ]) {
    assert.equal(isAllowedDependencyPath(path), false, path)
  }
})

test('accepts Dependabot commits and skips only the synthetic checkout merge', () => {
  const parents = ['b'.repeat(40), bot.hash]
  const merge = {
    hash: 'c'.repeat(40), authorEmail: 'human@example.com', parents,
    subject: `Merge ${parents[1]} into ${parents[0]}`, syntheticMerge: true,
  }
  assert.deepEqual(checkDependencyAutomation({ commits: [merge, bot], paths }), { ok: true, errors: [] })
  assert.equal(checkDependencyAutomation({ commits: [{ ...merge, syntheticMerge: false }, bot], paths }).ok, false)
  assert.equal(checkDependencyAutomation({ commits: [{ ...merge, subject: 'Merge feature' }, bot], paths }).ok, false)
})

test('rejects human commits and spoofed author names', () => {
  for (const commit of [
    { ...bot, authorEmail: 'human@example.com' },
    { ...bot, authorName: 'dependabot[bot]', authorEmail: 'spoof@example.com' },
  ]) {
    assert.equal(checkDependencyAutomation({ commits: [bot, commit], paths }).ok, false)
  }
})

test('rejects forbidden changes even with Dependabot authorship', () => {
  assert.equal(checkDependencyAutomation({ commits: [bot], paths: ['.github/CODEOWNERS'] }).ok, false)
})

test('rejects empty commit and path lists', () => {
  assert.equal(checkDependencyAutomation({ commits: [], paths }).ok, false)
  assert.equal(checkDependencyAutomation({ commits: [bot], paths: [] }).ok, false)
  assert.equal(checkDependencyAutomation().errors.length, 2)
})
