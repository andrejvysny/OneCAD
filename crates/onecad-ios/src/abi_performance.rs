//! Additive, opt-in host timing endpoints; no document state or wake effects.
use crate::abi::{OpaqueApp, check_size, read_struct, with_app};
use crate::app::Failure;
use crate::ffi::Status;
use onecad_ui::performance::{self, Stage};

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Record {
    pub struct_size: u32,
    pub stage: u32,
    pub correlation: u64,
    pub duration_ns: u64,
    pub a: u64,
    pub b: u64,
    pub c: u64,
}

#[repr(C)]
pub struct ExportStatus {
    pub struct_size: u32,
    pub enabled: u32,
    pub buffered: u64,
    pub dropped: u64,
    pub exported: u64,
    pub in_flight: u64,
    pub export_errors: u64,
}

/// # Safety
/// `app` is NULL or live; `record` is readable for its declared size.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn onecad_ios_performance_record(
    app: *mut OpaqueApp,
    record: *const Record,
) -> Status {
    // SAFETY: the caller supplies a live handle and a readable size-tagged record.
    unsafe {
        with_app(app, |_| {
            let record = read_struct(record)?;
            let stage = Stage::host(record.stage).ok_or_else(|| {
                Failure::new(Status::InvalidArgument, "unknown host performance stage")
            })?;
            performance::record_duration(
                stage,
                record.correlation,
                [record.a, record.b, record.c],
                record.duration_ns,
            );
            Ok(())
        })
    }
}

/// # Safety
/// `app` is NULL or live; `out` is writable for its declared size.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn onecad_ios_performance_status(
    app: *mut OpaqueApp,
    out: *mut ExportStatus,
) -> Status {
    // SAFETY: the caller supplies a live handle and a writable size-tagged output.
    unsafe {
        with_app(app, |_| {
            check_size(out.cast_const())?;
            let status = performance::status();
            out.write_unaligned(ExportStatus {
                struct_size: std::mem::size_of::<ExportStatus>() as u32,
                enabled: u32::from(status.enabled),
                buffered: status.buffered.map_or(u64::MAX, |count| count as u64),
                dropped: status.dropped,
                exported: status.exported,
                in_flight: status.in_flight,
                export_errors: status.export_errors,
            });
            performance::request_export();
            Ok(())
        })
    }
}
