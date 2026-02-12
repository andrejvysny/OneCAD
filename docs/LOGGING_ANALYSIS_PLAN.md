# Logging Analysis and Improvement Plan

## Current Logging State (Codebase Audit)

### Existing behavior
- Logging is currently inconsistent and mostly ad-hoc (`qWarning`, scattered `qDebug`), with no centralized structure.
- Startup/shutdown previously used `std::cout`/`std::cerr` in `main.cpp` and `Application.cpp`, bypassing Qt message routing.
- There was no global message handler, no per-run log file management, and no consistent formatting with timestamp/thread/category/source location.

### Observed message distribution
- Most existing logs are warnings and debug statements in rendering, sketch serialization/validation, and some I/O paths.
- No runtime-generated log files are created by default.
- No single point existed for controlling verbosity between debug and release behavior.

## Implemented Improvements

### Centralized logging subsystem
A new app-level logging subsystem was added:
- `src/app/Logging.h`
- `src/app/Logging.cpp`

Capabilities:
1. Installs a global Qt message handler (`qInstallMessageHandler`).
2. Writes all Qt log messages to both:
   - stdout/stderr (severity-based)
   - timestamped per-run `.log` file
3. Creates log folder automatically in app data (`AppLocalDataLocation/logs`), or uses `ONECAD_LOG_DIR` if provided.
4. Creates unique run file names with timestamp + PID:
   - `onecad_yyyyMMdd_HHmmss_zzz_<pid>.log`
5. Uses detailed structured formatting for each entry:
   - ISO timestamp with milliseconds
   - severity
   - thread id
   - category
   - file:line
   - function
   - message text
6. Supports debug verbosity policy:
   - Debug build: enables debug messages
   - Release build: suppresses debug by default
   - Override via env var `ONECAD_LOG_DEBUG=1|true|yes|on`

### Startup/shutdown lifecycle instrumentation
- `src/main.cpp` now initializes logging before `QApplication` creation.
- Startup flow logs key decisions/events:
  - build mode and argc
  - OpenGL default surface format parameters
  - app initialization result
  - OCCT version
  - event loop enter/exit code
- `src/app/Application.cpp` now logs initialization/shutdown decisions and metadata setup.

## Rollout Plan (Iterative)

### Phase 1 (completed)
- [x] Add centralized message handler and per-run log file generation.
- [x] Ensure dual sink output (console + file).
- [x] Add startup and application lifecycle logs.

### Phase 2 (recommended next)
- [ ] Introduce explicit logging categories in key modules (`onecad.io`, `onecad.render`, `onecad.ui.viewport`, `onecad.core.solver`).
- [ ] Add structured decision logs for user-driven workflows (tool activation, import/export attempts, sketch mode transitions).
- [ ] Standardize warning/error context payloads (object IDs, operation IDs, file paths).

### Phase 3 (recommended hardening)
- [x] Add 30-day + 30-file log retention policy with automatic pruning.
- [x] Keep all fields in logs (no redaction required for current app data profile).
- [x] Support selected debug categories in release mode.

## Applied Decisions

1. **Retention policy applied:** auto-prune by both age and count (`30 days` and `30 files`).
2. **PII policy:** no redaction; include full logging fields.
3. **Release verbosity:** allow selected debug categories while default debug remains off globally.
4. **Default log location:** platform-specific app data directory (`AppLocalDataLocation/logs`).
5. **DEBUG telemetry scope:** capture broad UI event stream and user interactions in debug mode.
6. **Crash diagnostics:** enabled via fatal-path abort handling and flush-on-write behavior; further OS-level signal hooks can be layered later if needed.

