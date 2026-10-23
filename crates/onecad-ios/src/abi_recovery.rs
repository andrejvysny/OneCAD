//! Background recovery completion is polled independently of rendering.
use crate::{
    abi::{self, OpaqueApp},
    app::{Failure, Outcome},
    ffi::{RecoverySnapshot, Status},
};

/// # Safety
/// `app` is a live app handle; `out` is NULL or writable for its declared size.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn onecad_ios_recovery_snapshot(
    app: *mut OpaqueApp,
    out: *mut RecoverySnapshot,
) -> Status {
    // SAFETY: the ABI caller supplies a live handle and a size-tagged output.
    unsafe {
        abi::with_app(app, |app| {
            abi::check_size(out)?;
            let (generation, status, results) =
                app.recovery
                    .as_ref()
                    .map_or((0, 1, Vec::new()), |recovery| {
                        let results = recovery.flush.results();
                        let status = if results.iter().any(|result| {
                            result.outcome == onecad_ui::host::RecoveryStatus::Pending
                        }) {
                            0
                        } else if results.iter().any(|result| {
                            result.outcome == onecad_ui::host::RecoveryStatus::Expired
                        }) {
                            2
                        } else if results.iter().any(|result| {
                            result.outcome == onecad_ui::host::RecoveryStatus::Cancelled
                        }) {
                            3
                        } else {
                            1
                        };
                        (recovery.generation, status, results)
                    });
            app.recovery_json = std::ffi::CString::new(
                serde_json::to_string(&results)
                    .map_err(|error| Failure::new(Status::Internal, error.to_string()))?,
            )
            .map_err(|error| Failure::new(Status::Internal, error.to_string()))?;
            out.write(RecoverySnapshot {
                struct_size: std::mem::size_of::<RecoverySnapshot>() as u32,
                status,
                generation,
                documents_json: app.recovery_json.as_ptr(),
            });
            Ok(())
        })
    }
}

/// # Safety
/// `app` is NULL or a live app handle, on its creating thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn onecad_ios_recovery_finish_waiting(
    app: *mut OpaqueApp,
    generation: u64,
    reason: u32,
) -> Status {
    // SAFETY: with_app checks handle ownership and contains panics.
    unsafe { abi::with_app(app, |app| finish(app.recovery.as_ref(), generation, reason)) }
}

fn finish(
    recovery: Option<&crate::lifecycle::BackgroundRecovery>,
    generation: u64,
    reason: u32,
) -> Outcome {
    if reason > 1 {
        return Err(Failure::new(
            Status::InvalidArgument,
            "unknown recovery completion reason",
        ));
    }
    let recovery = recovery
        .filter(|recovery| recovery.generation == generation)
        .ok_or_else(|| Failure::new(Status::State, "stale recovery generation"))?;
    recovery.flush.finish_waiting(reason == 0);
    Ok(())
}
