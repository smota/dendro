#!/usr/bin/env node
import { execFileSync } from 'node:child_process'

const DEPENDABOT_EMAIL = '49699333+dependabot[bot]@users.noreply.github.com'

export function isAllowedDependencyPath(path) {
  if (typeof path !== 'string') return false
  return /^(?:Cargo\.(?:toml|lock)|crates\/[A-Za-z0-9_-]+\/Cargo\.toml|\.github\/workflows\/[A-Za-z0-9_.-]+\.yml)$/.test(path)
}

export function checkDependencyAutomation({ commits = [], paths = [] } = {}) {
  const errors = []
  if (!commits.length) errors.push('dependency automation requires at least one commit')
  if (!paths.length) errors.push('dependency automation requires at least one changed path')
  for (const commit of commits) {
    // Only the checkout's synthetic PR merge is exempt, never a branch merge.
    if (
      commit.syntheticMerge === true &&
      commit.parents?.length === 2 &&
      commit.subject === `Merge ${commit.parents[1]} into ${commit.parents[0]}`
    ) continue
    if (commit.authorEmail !== DEPENDABOT_EMAIL) {
      errors.push(`commit ${commit.hash} is not authored by Dependabot`)
    }
  }
  for (const path of paths) {
    if (!isAllowedDependencyPath(path)) errors.push(`dependency automation cannot write ${path}`)
  }
  return { ok: errors.length === 0, errors }
}

export function main() {
  const errors = []
  try {
    if (!process.env.BASE_REF) throw new Error('BASE_REF is required')
    const base = `origin/${process.env.BASE_REF}`
    const git = (args) => execFileSync('git', args, { encoding: 'utf8' }).trim()
    const head = git(['rev-parse', 'HEAD'])
    const commits = git(['log', '--format=%H%x09%ae%x09%P%x09%s', `${base}..HEAD`])
      .split('\n')
      .filter(Boolean)
      .map((line) => {
        const [hash, authorEmail, parentText, ...subjectParts] = line.split('\t')
        return {
          hash,
          authorEmail,
          parents: parentText ? parentText.split(' ') : [],
          subject: subjectParts.join('\t'),
          syntheticMerge: hash === head && hash === process.env.GITHUB_SHA &&
            /^refs\/pull\/\d+\/merge$/.test(process.env.GITHUB_REF ?? ''),
        }
      })
    const paths = git(['diff', '--name-only', '--no-renames', `${base}...HEAD`])
      .split('\n')
      .filter(Boolean)
    errors.push(...checkDependencyAutomation({ commits, paths }).errors)
    try {
      execFileSync(process.execPath, [
        'scripts/validate-path-ownership.mjs', '--seat', 'dev.build', '--paths', paths.join(','),
      ], { encoding: 'utf8', stdio: ['pipe', 'pipe', 'pipe'] })
    } catch (error) {
      errors.push(error.stdout?.trim() || error.message)
    }
  } catch (error) {
    errors.push(error.message)
  }
  process.stdout.write(`${errors.length ? 'BLOCKED' : 'READY'}\n${errors.join('\n')}\n`)
  return errors.length ? 1 : 0
}

if (process.argv[1]?.endsWith('validate-dependency-automation.mjs')) {
  process.exitCode = main()
}
