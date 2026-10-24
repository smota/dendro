//! T24-52 cleanup admission. Reclamation requires separate disposal authority;
//! this API never derives that authority from Unix unlink semantics or a path.
#![deny(unsafe_code)]

use super::*;

/// A verified cleanup request was refused. No files or references were changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CleanupRefusal {
    pub reason: &'static str,
    pub current_manifest_reference: bool,
    pub immediate_predecessor_reference: bool,
    pub live_reader_reference: bool,
    pub disposal_authority_present: bool,
}

/// Request cleanup of the exact bundle retained by an actual reader capability.
/// Canonical writer/publisher locks serialize reference inspection with publication.
/// No unreference/disposal authorization exists in this API: every valid request
/// is refused, and invalid custody fails before producing a refusal receipt.
/// This is reference protection, not an implementation of bundle reclamation.
pub fn request_retained_bundle_cleanup(selected: &SelectedRead) -> Result<CleanupRefusal> {
    let binding = &selected.binding;
    let writer = name(&binding.record.writer)?;
    let publisher = name(&binding.record.publisher)?;
    let writer_id =
        p::canonical_identity_from_persisted_record(binding.record.writer_identity.into());
    let publisher_id =
        p::canonical_identity_from_persisted_record(binding.record.publisher_identity.into());
    p::with_exclusive_lock(&binding.root, &writer, &writer_id, |_| {
        p::with_exclusive_lock(&binding.root, &publisher, &publisher_id, |_| {
            Ok((|| -> Result<CleanupRefusal> {
                let current_binding = reopen_binding(binding)?;
                if current_binding.record.parent != binding.record.parent
                    || current_binding.record.writer_identity != binding.record.writer_identity
                    || current_binding.record.publisher_identity
                        != binding.record.publisher_identity
                {
                    return Err(anyhow!("cleanup binding changed under canonical locks"));
                }
                selected.custody.bundle().check_anchor()?;
                validate_bundle(&selected.custody)?;
                let current = selected_from_binding(current_binding)?;
                let target = Pointer::try_from(selected.selection())?;
                let current_reference = Pointer::try_from(current.selection())? == target;
                let predecessor = if current.binding.generation > 0 {
                    let state: PublicationState = read_record(
                        &current.binding.root,
                        &state_name(&current.binding.hash, current.binding.generation),
                    )?;
                    state.previous == Some(target)
                } else {
                    false
                };
                // Holding `selected` retains the verified bundle and its file pins.
                // Even a target absent from the newest state remains a live reader.
                Ok(CleanupRefusal {
                    reason: "referenced_snapshot_retained_no_disposal_authority",
                    current_manifest_reference: current_reference,
                    immediate_predecessor_reference: predecessor,
                    live_reader_reference: true,
                    disposal_authority_present: false,
                })
            })())
        })
    })?
}
