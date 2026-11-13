#!/usr/bin/env node
import { execFileSync } from 'node:child_process'
import { appendFileSync } from 'node:fs'
import { pathToFileURL } from 'node:url'

const proofPath = /^(?:crates\/dendro-core\/src\/(?:storage[^/]*|migration[^/]*)(?:\/.*)?|crates\/dendro-core\/src\/platform_v1\/.*|crates\/(?:dendro-windows-private-io|dendro-platform-policy)\/.*|crates\/dendro-core\/tests\/(?:[^/]*migration[^/]*|native_foundation_acceptance[^/]*)|scripts\/historical_migration_proof\.py|scripts\/fixtures\/.*|scripts\/foundation-ci-[^/]*|scripts\/ci-scope\.mjs|\.github\/workflows\/foundation\.yml|\.agentflow\/oracles\/.*)$/

function changedDatabasePackage(diff = '') {
  // -U1 keeps the package name as context for version-only lockfile bumps.
  // A name from a separate hunk must never label an unrelated package change.
  return diff.split(/^@@.*@@.*$/m).slice(1).some((hunk) =>
    /^[ +\-]name = "(?:duckdb|libduckdb-sys|rusqlite|libsqlite3-sys)"$/m.test(hunk) &&
    /^[+\-](?![+\-])\S/m.test(hunk))
}

export function decideScope({ eventName, ref, baseRef, headRef, requestedScope, paths, cargoLockDiff } = {}) {
  const result = (scope, reason) => ({ scope, reason })
  if (eventName === 'workflow_dispatch') return result(['fast', 'full'].includes(requestedScope) ? requestedScope : 'full', 'manual workflow scope')
  if (eventName === 'push') return result(ref === 'refs/heads/development' ? 'fast' : 'full', 'push branch scope')
  if (eventName !== 'pull_request') return result('full', 'unknown event; full proof required')
  if (baseRef === 'main' || baseRef?.startsWith('rc/') || /^(?:rc|hotfix)\//.test(headRef ?? '')) return result('full', 'release or hotfix pull request')
  if (baseRef !== 'development') return result('full', 'unknown pull request base; full proof required')
  if (!Array.isArray(paths) || paths.length === 0) return result('full', 'changed paths unavailable or empty; full proof required')
  if (paths.some((path) => proofPath.test(path))) return result('full', 'foundation proof path changed')
  if (changedDatabasePackage(cargoLockDiff)) return result('full', 'database dependency changed')
  if (paths.every((path) => path.startsWith('docs/') || /^[^/]+\.md$/.test(path))) return result('skip', 'documentation-only pull request')
  return result('fast', 'routine development pull request')
}

export function main() {
  const { EVENT_NAME: eventName, REF: ref, BASE_REF: baseRef, HEAD_REF: headRef, REQUESTED_SCOPE: requestedScope } = process.env
  let decision
  try {
    let paths
    let cargoLockDiff
    if (eventName === 'pull_request') {
      if (!baseRef) throw new Error('BASE_REF is required')
      const range = `origin/${baseRef}...HEAD`
      paths = execFileSync('git', ['diff', '--name-only', range], { encoding: 'utf8' }).trim().split('\n').filter(Boolean)
      cargoLockDiff = execFileSync('git', ['diff', '-U1', range, '--', 'Cargo.lock'], { encoding: 'utf8' })
    }
    decision = decideScope({ eventName, ref, baseRef, headRef, requestedScope, paths, cargoLockDiff })
  } catch (error) {
    decision = { scope: 'full', reason: `diff unavailable; full proof required: ${error.message}` }
  }
  if (process.env.GITHUB_OUTPUT) appendFileSync(process.env.GITHUB_OUTPUT, `scope=${decision.scope}\n`)
  console.log(`${decision.scope}: ${decision.reason}`)
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) main()
