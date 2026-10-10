import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'

export const DEFAULT_ORCHESTRATION_MODEL = Object.freeze({
  lifecycle: 'dual-gate',
  preCodeReview: {
    enabled: true,
    gate: 'specification-sparring',
    targetPhases: ['architect', 'implementation-planner'],
    blockingSeverity: ['blocker', 'structural'],
  },
  concurrency: {
    maxWriters: 1,
    maxAdvisors: 3,
    isolatedWorktreeRequired: false,
  },
})

export const DEFAULT_EXECUTION_POLICY = Object.freeze({
  escalation: {
    maxAttemptsPerTier: 2,
    tierProgression: ['balanced', 'deep-reasoning', 'architect-lead'],
    suspendOnRepeatedRejections: 2,
  },
  fallbackCascade: {
    primary: 'external-cli',
    onQuotaExhausted: 'inner-subagent',
    onSubagentExhausted: 'human-gate',
    recordDegradationInEvidence: true,
  },
})

export const DEFAULT_MODEL_CATALOG = Object.freeze({
  tiers: {
    'architect-lead': { capabilities: ['high-context', 'formal-logic', 'deep-reasoning'] },
    'deep-reasoning': { capabilities: ['adversarial-audit', 'security-analysis'] },
    balanced: { capabilities: ['fast-implementation', 'unit-testing'] },
    lightweight: { capabilities: ['fixtures', 'formatting', 'inventory'] },
  },
  roleAssignments: {
    coordinator: 'architect-lead',
    'adversarial-reviewer': 'deep-reasoning',
    developer: 'balanced',
    'fixture-builder': 'lightweight',
  },
})

export const DEFAULT_HARNESS_PARAMETERS = Object.freeze({
  shellRunner: {
    windowsStrategy: 'cmd-type-stream',
    posixStrategy: 'direct-pipe',
    stdinTimeoutMs: 15000,
  },
  limits: {
    envelopeByteCapKb: 48,
    maxReviewIterations: 12,
    largeFileScriptThresholdLines: 800,
  },
  scratchPath: '.agent-runs/scratch',
})

function readJsonFile(path) {
  try {
    if (existsSync(path)) {
      return JSON.parse(readFileSync(path, 'utf8'))
    }
  } catch {
    // Ignore read/parse error and return null
  }
  return null
}

export function loadHarnessIntelligence(targetDir = process.cwd()) {
  const agentflowDir = join(targetDir, '.agentflow')
  const rootConfigPath = join(targetDir, 'agent-workflow.config.json')
  const rootConfig = readJsonFile(rootConfigPath) || {}

  // 1. Orchestration Model
  const orchestrationModel =
    readJsonFile(join(agentflowDir, 'orchestration-model.json')) ||
    rootConfig.orchestrationModel ||
    DEFAULT_ORCHESTRATION_MODEL

  // 2. Execution Policy
  const executionPolicy =
    readJsonFile(join(agentflowDir, 'execution-policy.json')) ||
    rootConfig.executionPolicy ||
    DEFAULT_EXECUTION_POLICY

  // 3. Model Catalog
  const modelCatalog =
    readJsonFile(join(agentflowDir, 'model-catalog.json')) ||
    rootConfig.modelCatalog ||
    DEFAULT_MODEL_CATALOG

  // 4. Harness Parameters
  const harnessParameters =
    readJsonFile(join(agentflowDir, 'harness-parameters.json')) ||
    rootConfig.harnessParameters ||
    DEFAULT_HARNESS_PARAMETERS

  return {
    orchestrationModel,
    executionPolicy,
    modelCatalog,
    harnessParameters,
  }
}

const SCAFFOLD_PILLARS = [
  { file: 'orchestration-model.json', data: DEFAULT_ORCHESTRATION_MODEL },
  { file: 'execution-policy.json', data: DEFAULT_EXECUTION_POLICY },
  { file: 'model-catalog.json', data: DEFAULT_MODEL_CATALOG },
  { file: 'harness-parameters.json', data: DEFAULT_HARNESS_PARAMETERS },
]

// The exact bytes a scaffold writes, keyed by project-relative path, so a preview can show them.
export function harnessScaffoldFiles() {
  return Object.fromEntries(
    SCAFFOLD_PILLARS.map(({ file, data }) => [
      `.agentflow/${file}`,
      `${JSON.stringify(data, null, 2)}\n`,
    ]),
  )
}

export function scaffoldHarnessIntelligence(targetDir = process.cwd(), { force = false } = {}) {
  const agentflowDir = join(targetDir, '.agentflow')
  if (!existsSync(agentflowDir)) {
    mkdirSync(agentflowDir, { recursive: true })
  }
  const created = []
  const existing = []
  for (const { file, data } of SCAFFOLD_PILLARS) {
    const fullPath = join(agentflowDir, file)
    if (!existsSync(fullPath) || force) {
      writeFileSync(fullPath, `${JSON.stringify(data, null, 2)}\n`, 'utf8')
      created.push(file)
    } else {
      existing.push(file)
    }
  }
  return { agentflowDir, created, existing }
}

export function inspectHarnessIntelligence(targetDir = process.cwd()) {
  const agentflowDir = join(targetDir, '.agentflow')
  const pillars = [
    'orchestration-model.json',
    'execution-policy.json',
    'model-catalog.json',
    'harness-parameters.json',
  ]
  const status = {}
  let configuredCount = 0
  for (const pillar of pillars) {
    const fullPath = join(agentflowDir, pillar)
    const exists = existsSync(fullPath)
    if (exists) configuredCount += 1
    status[pillar] = {
      path: fullPath,
      exists,
    }
  }
  return {
    agentflowDir,
    configuredCount,
    totalPillars: pillars.length,
    status,
    loaded: loadHarnessIntelligence(targetDir),
  }
}
