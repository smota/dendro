# Phase 0 — #46 source-output alias protection

Decision proceed as a separate source-protection defect. pm.manager /
pm-manager@dendro-pm, Codex / codex-cli / local-cli / current-session,
2026-10-14. Key46-phase0-source-output-alias.
Goal https://github.com/smota/dendro/issues/46
Origin #30 QA; scope request qitem-20261014215651-c35e73b701d15035.
Framing/evidence goals/source-output-alias/issue.md.

When I accidentally select a live source as analytics output, I want ingest to
refuse before write-capable source access or modification, so a failed command
cannot disturb the fleet's source data. Push is observed source WAL write-capable
opens and SHM changes; pull is a safe refusal; anxiety is a diagnostic masking
source mutation; inertia is an existing alias guard that presently runs too late.
No commercial/emotional/social evidence inferred.

Classify as a defect against accepted R1 AC12/#30 source protection, not a new
outcome. Baseline presence and successful wording tests cannot waive it. AC12's
SHM exception does not permit source WAL O_RDWR|O_CREAT. Keep #30 wording criteria
and its retained quick_check classification limit separate from this finding.
#30 currentPhase4/pending QA remains held; no phase5/8 acceptance or synthetic
phase return. #46 independent QA/review and composed-candidate reverification
are prerequisite evidence for a later unqualified #30 pass.

Analyst pins public refusal and alias identity boundary under existing source/
containment guarantees, original main/WAL/SHM access/state evidence, distinct
output success, root-binding/key/privacy and snapshot-policy regression. Use
synthetic sources only; no owner keys or real-source mutation tests. Architect
owns assurance, technical solution and isolation; do not prescribe guard order
or files as the implementation. QA/review retain independent candidate gates.
Missing syscall/state evidence remains unknown; hashes alone are insufficient.

No universal filesystem-race guarantee, generic hardening, retry-policy change,
metrics/schema scope expansion or #12/#13 acceptance. v0.2.0 minor local only;
no main edit, push, PR, merge, tag or publication. Existing holds remain.
Read verified current #30 phase4, architect request, synthetic comparative
findings/candidate operations, R1 AC12 and frozen #30 contract. Manifest details
arrived in architect's follow-up and remain independent QA evidence, not PM
reverification of all67 files. Analyst should bind final evidence pins.

Next owner pm.analyst: record Phase1 and return contract/digest/unknowns to
pm.manager. Only accepted Phase1 via governed handoff deliver may authorize
delivery for #46. Source protection merits architect assurance assessment;
product does not lower assurance or authorize code from this Phase0 record.
