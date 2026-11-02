# Dendro v0.10.0

## OpenRig 0.7 contract

Dendro classifies an OpenRig sqlite source before it writes facts. The classes are supported, additive, drifted, and unsupported. A missing `events` or `queue_transitions` table, a missing required column, or a type mismatch is unsupported: Dendro prints one operator line and refuses before facts. An unknown column or a migration newer than the pin is additive and is still ingested. A missing migration table, or a missing non-required table, is drifted. Drifted sources still ingest unless `DENDRO_OPENRIG_CONTRACT=strict`. Any other value, including an unset variable, keeps the warning.

`queue.handed_off` is one queue fact. A matching `queue_transitions` row is not counted again. Stateless `seat.activity_changed` does not increment the unknown-event count. Unknown event kinds do.

The operator line is:

```text
dendro: openrig contract=<supported|additive|drifted|unsupported> rig=<version|unknown> migration=<name|none> unknown_events=<n> dropped=<n> reason=<short>
```

Issue #109 is closed on `development`. It is not on `main`.

## Windows schema-5 cutover

Schema-5 I/O errors now name the step. The hosted failure was `publish: Access is denied. (os error 5)`. `FlushFileBuffers` needs `GENERIC_WRITE`, and opening a directory in that helper needs `FILE_FLAG_BACKUP_SEMANTICS`. Unix still uses a read-only file. Backup membership, alias handling, and rollback guards are unchanged.

On windows-2022 run 41440996525, `cargo test -p dendro-core --test migration_v5` passed 7 tests, including `exclusive_backup_pointer_cutover_and_rollback` and `windows_provenance_aliases_backup_once_and_preserve_rollback`. Issue #106 is closed.

## Sharing violation is still open

`native_staging_rework` still fails on the hosted elevated runner at the Administrators-grant `publish_replace` with Win32 code 32. On that error only, the test records a Restart Manager note for the source and destination. An empty scan is `none_reported`. The publish path and the audited `ffi.rs` boundary are unchanged. Issue #91 stays open.

## Source basis

These notes cover `development` after the v0.9.1 preparation (`b35452b`) through `736dd24`, plus this metadata commit. The newest git tag remains `v0.9.0`. This preparation does not merge to `main` and does not publish a GitHub release.
