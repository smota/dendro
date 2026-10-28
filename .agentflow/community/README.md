# Dendro Community Interaction & Triage Playbook

This directory (`.agentflow/community/`) holds **staged community draft responses, automated bug reproduction receipts, and external Pull Request security/quality audits** for `smota/dendro`.

In accordance with `sdlc.config.json` (`"externalActionRequiresHumanApproval": true`), Dendro separates **autonomous internal triage/staging** from **human-gated external GitHub actions**. No agent seat may post public comments, approve untrusted fork CI runs, or merge external PRs without explicit approval from `@smota`.

---

## 1. Authority Matrix: Autonomous vs. Human-Gated Actions

### 1.1. Autonomous Actions (Allowed Without Human Gate)

| Seat | Autonomous Scope | Output / Artifact Surface |
| :--- | :--- | :--- |
| **`pm.manager`** (`pm-manager@dendro-pm`) | 1. Read and classify inbound GitHub Issues & Discussions.<br>2. Apply canonical taxonomy labels defined in `.github/labels.json` (`type`, `state:backlog`, etc.; never `agent:*`).<br>3. Cross-reference duplicates and backlog items via `S0` (`gemini-3.8-flash` `low`).<br>4. Stage draft responses via `S1` (`grok-4.7-build-fast` `medium`). | `.agentflow/community/draft-issue-<N>.md`<br>`.agentflow/community/draft-discussion-<N>.md` |
| **`dev.qa`** (`dev-qa@dendro-dev`) | 1. Execute deterministic bug reproduction in a clean, isolated sandbox/worktree using reported steps.<br>2. Capture sanitized stack traces, environment metadata, and pass/fail reproduction status.<br>3. Never execute unverified shell payloads or untrusted scripts outside sandbox containment. | `.agentflow/community/issue-<N>.md` |
| **`rev.review`** (`rev-review@dendro-dev`) | 1. Extract external PR diff context via `S0` (`gemini-3.8-flash` `low`).<br>2. Perform orthogonal 4-eyes security, license, SQL-injection, path-traversal, and secret-redaction audit (`claude-sonnet-5-5` `medium`–`high`).<br>3. Record structured audit verdict locally before any CI or public review action. | `.agentflow/community/pr-<N>-audit.md` |

### 1.2. Strictly Human-Gated Actions (`@smota` Approval Required)

Enforced by `sdlc.config.json` (`actionPolicy.externalActionRequiresHumanApproval: true`):

- **Posting Public Replies:** Publishing any comment or reply on GitHub Issues, GitHub Discussions, or Pull Requests.
- **Approving Fork CI Runs:** Triggering or approving GitHub Actions workflow runs for external fork Pull Requests.
- **Assigning Sprint Milestones:** Binding an issue or PR to a release milestone or committing delivery dates publicly.
- **Merging External Pull Requests:** Merging any contributor PR into `development`, `rc/vX.Y.Z`, or `main`.

---

## 2. End-to-End Triage Workflows

### 2.1. Inbound Issue Workflow
1. **Classification & Labeling (`pm.manager`):** Inspects the issue, applies validated labels from `.github/labels.json`, and checks for existing duplicates or related capabilities in `.agentflow/capabilities.catalog.yaml`.
2. **Bug Reproduction (`dev.qa`, if `bug`):** If the issue reports a defect, `pm.manager` routes a reproduction check to `dev.qa`, which runs a clean sandbox reproduction and writes `.agentflow/community/issue-<N>.md`.
3. **Draft Staging (`pm.manager`):** Synthesizes findings into `.agentflow/community/draft-issue-<N>.md` and notifies `@smota` for approval before posting.

### 2.2. Inbound Discussion Workflow
1. **Classification (`pm.manager`):** Categorizes the discussion (Q&A, feature idea, architecture question, or showcase) and maps it to relevant docs or `docs/BACKLOG.md`.
2. **Draft Staging (`pm.manager`):** Stages a concise, technically grounded response in `.agentflow/community/draft-discussion-<N>.md` for `@smota` review.

### 2.3. External Pull Request Workflow
1. **Static Security & Quality Audit (`rev.review`):** Before approving fork CI or commenting publicly, `rev.review` uses `S0` to extract the diff and audits the PR against Dendro's Rust engineering standards (`#![deny(unsafe_code)]`, parameterized DuckDB/SQLite queries, path containment, secret sanitization), writing `.agentflow/community/pr-<N>-audit.md`.
2. **Maintainer Decision (`@smota`):** `@smota` reviews `.agentflow/community/pr-<N>-audit.md` to decide whether to approve fork CI runs, post review feedback, or integrate the PR into `development`.

---

## 3. Artifacts & Staged Templates

### 3.1. Staged Issue Reply Template (`.agentflow/community/draft-issue-<N>.md`)

```markdown
---
kind: community-draft-issue
issueNumber: <N>
issueUrl: https://github.com/smota/dendro/issues/<N>
author: "@<github-handle>"
classifiedBy: pm-manager@dendro-pm
labelsApplied:
  - bug
  - state:backlog
reproductionReceipt: .agentflow/community/issue-<N>.md
humanGate:
  requiredApprover: "@smota"
  externalActionRequiresHumanApproval: true
  status: pending-approval # pending-approval | approved | posted | rejected
  recommendedMilestone: null # Requires @smota approval to assign
---

# Triage Summary (Internal for `@smota`)
- **Classification:** <Bug | Feature Request | DX | Question>
- **Reproduction Status:** <Confirmed | Not Reproducible | Needs Info — see .agentflow/community/issue-<N>.md>
- **Catalog / Backlog Link:** <Capability ID or docs/BACKLOG.md reference>
- **Proposed Action:** <Post staged reply below | Request minimal repro | Promote to G1 analyst framing>

---

# Proposed Public Reply (Awaiting `@smota` Approval)

Thanks for reporting this, @<github-handle>.

<Concise, evidence-backed response summarizing reproduction findings, workaround if any, or how this fits Dendro's local-first architecture.>
```

### 3.2. Staged Discussion Reply Template (`.agentflow/community/draft-discussion-<N>.md`)

```markdown
---
kind: community-draft-discussion
discussionNumber: <N>
discussionUrl: https://github.com/smota/dendro/discussions/<N>
author: "@<github-handle>"
category: <Q&A | Ideas | General | Show and Tell>
classifiedBy: pm-manager@dendro-pm
labelsApplied:
  - exploratory
humanGate:
  requiredApprover: "@smota"
  externalActionRequiresHumanApproval: true
  status: pending-approval # pending-approval | approved | posted | rejected
---

# Discussion Context (Internal for `@smota`)
- **Core Question / Proposal:** <1-line summary of the user's intent>
- **Product Alignment:** <Alignment with .agentflow/PRODUCT.md and docs/BACKLOG.md>
- **Proposed Follow-Up:** <Answer only | Log candidate item in docs/BACKLOG.md>

---

# Proposed Public Reply (Awaiting `@smota` Approval)

Hi @<github-handle>, thanks for starting this discussion!

<Clear, technically accurate response referencing relevant Dendro docs, CLI commands, or roadmap direction without over-promising unapproved milestones.>
```

### 3.3. QA Bug Reproduction Receipt Format (`.agentflow/community/issue-<N>.md`)

```markdown
---
kind: community-bug-reproduction
issueNumber: <N>
verifiedBy: dev-qa@dendro-dev
sandboxEnvironment: clean-worktree
status: reproduced # reproduced | not-reproduced | blocked-needs-info
commitSha: <HEAD SHA tested>
---

# Reproduction Evidence
- **Command / Steps Executed:** `<exact deterministic command or test>`
- **Observed Behavior:** <summary + sanitized error/log excerpt>
- **Expected Behavior:** <expected contract>
- **Root-Cause Pointer:** `<file:line>` (extractive fact only)
```

### 3.4. External PR Audit Format (`.agentflow/community/pr-<N>-audit.md`)

```markdown
---
kind: community-pr-audit
prNumber: <N>
prUrl: https://github.com/smota/dendro/pull/<N>
auditedBy: rev-review@dendro-dev
s0ExtractorUsed: gemini-3.8-flash
reviewerModel: claude-sonnet-5-5
forkCiApprovalSafe: false # true | false (requires @smota action to approve run)
verdict: changes-requested # approved-for-ci | changes-requested | reject-security-risk
---

# Security & Guardrail Checklist
- [ ] No `unsafe` blocks (`#![deny(unsafe_code)]`)
- [ ] Parameterized DuckDB / SQLite queries only (`params![...]`)
- [ ] Path containment & symlink cycle checks preserved
- [ ] Secret & token redaction preserved (`sk-`, `ghp_`, `gho_`, `xai-`, Bearer)
- [ ] No unauthorized workflow, hook, or governance file mutations (`.github/**`, `sdlc.config.json`, `openrig/**`)

# Findings (`file:line`)
- `<file:line>` — <finding summary>
```
