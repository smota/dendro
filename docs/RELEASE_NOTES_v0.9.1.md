# Dendro v0.9.1

## Direct Operator Overview — Modo Direto

Issue #101 adds an evidence-backed Direct Operator Overview with four decision
cards, predefined question chips, a contextual drawer, and a toggle between
operator and analytical modes. The Context-Tax spotlight helps operators inspect
context overhead and the evidence behind it. Production repairs align the
persisted overview, scoped replay, API, and dashboard paths and avoid nested
transactions during overview evaluation.

## Windows reliability

- **CLI stack exhaustion:** CLI parsing and async runtime entry now run on a
  dedicated `dendro-runtime` thread with a 4 MiB stack, addressing Windows stack
  exhaustion reported as `0xc00000fd`.
- **Named pipe contention:** A bounded async retry loop handles
  `ERROR_PIPE_BUSY` (231) while opening a connection, with at most 25 retries and
  a 10 ms delay between attempts. Other open errors return immediately; requests
  are not replayed after delivery.
- **Synthetic fixture portability:** Windows overview fixtures use
  `create_user_private_root_with_choice` with `StorageRootChoice::UserProfile`
  so synthetic databases and keys reside in admitted private roots. This changes
  fixture placement without weakening native privacy checks.
- **Migration provenance:** Canonical file identity deduplicates discovered and
  explicit provenance paths, normalizing aliases with and without the Windows
  `\\?\` verbatim prefix. Backup names retain their source paths, and distinct
  files with colliding names still fail rather than overwrite a backup.

## Historical migration proof

The historical migration proof fixture explicitly uses DuckDB's `v1.5.6`
storage compatibility setting and verifies storage format **v68** in the file
header. This keeps the synthetic database readable by the historical binaries
used in the backward compatibility proof. The candidate's reported CLI version
is bound to its manifest version instead of a hard-coded historical release.
These are proof-harness changes, not a claim of compatibility with every DuckDB
storage format.

## Publication disclosure

Publication is authorized under the person's explicit override: **"force merge and deploy and conclude"**.
At PR #104 source merge into `development` (`5ac4d4d82e1ae0ce14d10b2397642330f92d1a83`), final required CI checks were pending and fresh independent QA and Phase 6 review acceptance were absent. This release does not claim completed acceptance by those gates or represent future CI conclusions.

## Source basis

These notes cover changes after `v0.9.0` (`3f1b29cc649f1d7f14cf147c0bcad5c31218ca88`)
through candidate source `f1191fcd9be9252ef200cf57323a0b7b191bf4df`.
The v0.9.1 preparation commit updates release metadata and documentation only.
