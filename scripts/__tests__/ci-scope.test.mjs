import assert from 'node:assert/strict'
import { test } from 'node:test'
import { decideScope } from '../ci-scope.mjs'

const pr = { eventName: 'pull_request', baseRef: 'development', headRef: 'feature/test', paths: ['crates/dendro-cli/src/main.rs'] }
const scope = (input) => decideScope({ ...pr, ...input }).scope

test('manual scope defaults full and accepts only fast or full', () => {
  for (const requestedScope of [undefined, '', 'skip', 'invalid', 'full']) assert.equal(scope({ eventName: 'workflow_dispatch', requestedScope }), 'full')
  assert.equal(scope({ eventName: 'workflow_dispatch', requestedScope: 'fast' }), 'fast')
})

test('push scope depends only on development branch', () => {
  assert.equal(scope({ eventName: 'push', ref: 'refs/heads/development' }), 'fast')
  for (const ref of ['refs/heads/main', 'refs/heads/rc/v1', undefined]) assert.equal(scope({ eventName: 'push', ref }), 'full')
})

test('release and hotfix pull requests always require full proof', () => {
  for (const baseRef of ['main', 'rc/v1']) assert.equal(scope({ baseRef, paths: ['README.md'] }), 'full')
  for (const headRef of ['rc/v1', 'hotfix/fix']) assert.equal(scope({ headRef, paths: ['README.md'] }), 'full')
})

test('unknown events, bases and missing paths fail closed', () => {
  for (const paths of [undefined, null, []]) assert.equal(scope({ paths }), 'full')
  assert.equal(scope({ eventName: 'schedule' }), 'full')
  assert.equal(scope({ baseRef: 'other' }), 'full')
})

test('each foundation proof path requires full proof', () => {
  for (const path of [
    'crates/dendro-core/src/storage.rs', 'crates/dendro-core/src/storage/read.rs',
    'crates/dendro-core/src/migrations.rs', 'crates/dendro-core/src/migration/one.rs',
    'crates/dendro-core/src/platform_v1/mod.rs', 'crates/dendro-windows-private-io/src/lib.rs',
    'crates/dendro-platform-policy/src/lib.rs', 'crates/dendro-core/tests/test_migration.rs',
    'crates/dendro-core/tests/native_foundation_acceptance.rs', 'scripts/historical_migration_proof.py',
    'scripts/fixtures/test.rs', 'scripts/foundation-ci-verify.sh', 'scripts/ci-scope.mjs',
    '.github/workflows/foundation.yml', '.agentflow/oracles/v080/frozen.json',
  ]) assert.equal(scope({ paths: [path] }), 'full', path)
})

test('docs-only skips while mixed docs and code and unrelated migration paths are fast', () => {
  assert.equal(scope({ paths: ['README.md', 'docs/guide.md'] }), 'skip')
  assert.equal(scope({ paths: ['README.md', 'crates/dendro-cli/src/main.rs'] }), 'fast')
  assert.equal(scope({ paths: ['scripts/migration-helper.mjs'] }), 'fast')
})

test('ordinary Dependabot lockfile bumps are fast; database packages require full', () => {
  for (const name of ['serde', 'duckdb', 'libduckdb-sys', 'rusqlite', 'libsqlite3-sys']) {
    const cargoLockDiff = `diff --git a/Cargo.lock b/Cargo.lock\n--- a/Cargo.lock\n+++ b/Cargo.lock\n@@ -10,3 +10,3 @@\n name = "${name}"\n-version = "1.0"\n+version = "1.1"\n source = "registry"\n`
    assert.equal(scope({ paths: ['Cargo.lock'], cargoLockDiff }), name === 'serde' ? 'fast' : 'full', name)
  }
})

test('added and removed database packages require full, unrelated hunks stay fast', () => {
  for (const prefix of ['+', '-']) assert.equal(scope({ paths: ['Cargo.lock'], cargoLockDiff: `@@ -1 +1 @@\n${prefix}name = "duckdb"\n` }), 'full')
  assert.equal(scope({ paths: ['Cargo.lock'], cargoLockDiff: '@@ -1 +1 @@\n name = "duckdb"\n@@ -20 +20 @@\n-old\n+new\n' }), 'fast')
})
