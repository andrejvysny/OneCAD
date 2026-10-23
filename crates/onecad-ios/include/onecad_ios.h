/* onecad_ios.h — C ABI between the OneCAD UIKit host and the shared Rust app.
 *
 * Contract: docs/ios/ARCHITECTURE.md. All functions except
 * onecad_ios_abi_version() must be called on the thread that created the app
 * (the UIKit main thread). Every struct begins with struct_size; a struct
 * smaller than the version this header defines is rejected with
 * ONECAD_IOS_ERR_INVALID_ARGUMENT. Inputs are copied before a call returns.
 * Pointers returned by the library stay valid until the next call of the same
 * function or onecad_ios_app_destroy, unless documented otherwise.
 */
#ifndef ONECAD_IOS_H
#define ONECAD_IOS_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#define ONECAD_IOS_ABI_VERSION 2u

typedef struct onecad_ios_app onecad_ios_app;

typedef enum onecad_ios_status {
    ONECAD_IOS_OK = 0,
    ONECAD_IOS_ERR_INVALID_ARGUMENT = 1,
    ONECAD_IOS_ERR_WRONG_THREAD = 2,
    ONECAD_IOS_ERR_STATE = 3,      /* call not valid in the current lifecycle state */
    ONECAD_IOS_ERR_PANIC = 4,      /* a Rust panic was contained; app is poisoned */
    ONECAD_IOS_ERR_TERMINAL = 5,   /* kernel relaunch required or app poisoned */
    ONECAD_IOS_ERR_OVERFLOW = 6,   /* bounded queue full; terminal events kept */
    ONECAD_IOS_ERR_INTERNAL = 7,
    ONECAD_IOS_ERR_UNAVAILABLE = 8 /* feature absent in this build or device */
} onecad_ios_status;

uint32_t onecad_ios_abi_version(void);

/* Any thread, any time between create and the return of destroy. Must only
 * schedule a frame on the main thread; must not call back into the library. */
typedef void (*onecad_ios_wake_fn)(void *ctx);

typedef struct onecad_ios_config {
    uint32_t struct_size;
    const char *documents_dir;   /* UTF-8 absolute paths, copied */
    const char *app_support_dir;
    const char *caches_dir;
    const char *tmp_dir;
    onecad_ios_wake_fn wake;
    void *wake_ctx;
    uint32_t flags;              /* reserved, 0 */
} onecad_ios_config;

onecad_ios_status onecad_ios_app_create(const onecad_ios_config *config,
                                        onecad_ios_app **out_app);
/* Cancels transient contacts, flushes autosave, retires the kernel with a
 * bounded grace and releases the surface. No callback runs after return. */
onecad_ios_status onecad_ios_app_destroy(onecad_ios_app *app);

/* ---- Surface ----------------------------------------------------------- */

typedef struct onecad_ios_insets {
    float top, left, bottom, right; /* logical points */
} onecad_ios_insets;

typedef struct onecad_ios_surface_desc {
    uint32_t struct_size;
    void *ca_metal_layer;        /* CAMetalLayer*, retained by the host while attached */
    uint32_t width_px, height_px;
    float scale;                 /* drawable pixels per logical point */
    onecad_ios_insets safe_area; /* logical points */
} onecad_ios_surface_desc;

onecad_ios_status onecad_ios_surface_attach(onecad_ios_app *app,
                                            const onecad_ios_surface_desc *desc);
/* Same layer, new size/scale/safe area (rotation, split view, keyboard). */
onecad_ios_status onecad_ios_surface_resize(onecad_ios_app *app,
                                            const onecad_ios_surface_desc *desc);
/* Releases GPU surface state; the layer may be destroyed after return. */
onecad_ios_status onecad_ios_surface_detach(onecad_ios_app *app);

/* ---- Frame ------------------------------------------------------------- */

#define ONECAD_IOS_FRAME_REDRAW_REQUESTED  (1u << 0) /* draw again soon */
#define ONECAD_IOS_FRAME_TEXT_INPUT_ACTIVE (1u << 1) /* show the keyboard */
#define ONECAD_IOS_FRAME_A11Y_CHANGED      (1u << 2) /* re-read a11y snapshot */
#define ONECAD_IOS_FRAME_HOST_REQUESTS     (1u << 3) /* poll host requests */
#define ONECAD_IOS_FRAME_KERNEL_TERMINAL   (1u << 4) /* relaunch required */
#define ONECAD_IOS_FRAME_PRESENTED         (1u << 5) /* a drawable was presented */

typedef struct onecad_ios_frame_result {
    uint32_t struct_size;
    uint32_t flags;
    double repaint_after_s;      /* < 0: no scheduled repaint */
    float text_cursor_x, text_cursor_y, text_cursor_h; /* points, when text active */
    uint32_t text_input_kind;    /* onecad_ios_text_kind */
} onecad_ios_frame_result;

/* Runs one egui frame and presents it. time_s: CACurrentMediaTime(). */
onecad_ios_status onecad_ios_frame(onecad_ios_app *app, double time_s,
                                   onecad_ios_frame_result *out);

/* ---- Touch and Pencil input ------------------------------------------- */

typedef enum onecad_ios_contact_kind {
    ONECAD_IOS_CONTACT_FINGER = 0,
    ONECAD_IOS_CONTACT_PENCIL = 1,
    ONECAD_IOS_CONTACT_INDIRECT_POINTER = 2 /* trackpad/mouse pointer */
} onecad_ios_contact_kind;

typedef enum onecad_ios_phase {
    ONECAD_IOS_PHASE_BEGAN = 0,
    ONECAD_IOS_PHASE_MOVED = 1,
    ONECAD_IOS_PHASE_STATIONARY = 2,
    ONECAD_IOS_PHASE_ENDED = 3,
    ONECAD_IOS_PHASE_CANCELLED = 4,
    ONECAD_IOS_PHASE_HOVER_MOVED = 5,  /* Pencil/pointer hover, no contact */
    ONECAD_IOS_PHASE_HOVER_EXITED = 6
} onecad_ios_phase;

typedef enum onecad_ios_provenance {
    ONECAD_IOS_SAMPLE_ACTUAL = 0,     /* the event's current touch */
    ONECAD_IOS_SAMPLE_COALESCED = 1,  /* earlier sample from coalescedTouches */
    ONECAD_IOS_SAMPLE_PREDICTED = 2,  /* predictedTouches; transient only */
    ONECAD_IOS_SAMPLE_ESTIMATION_UPDATE = 3 /* touchesEstimatedPropertiesUpdated */
} onecad_ios_provenance;

#define ONECAD_IOS_HAS_FORCE     (1u << 0)
#define ONECAD_IOS_HAS_ALTITUDE  (1u << 1)
#define ONECAD_IOS_HAS_AZIMUTH   (1u << 2)
#define ONECAD_IOS_HAS_ROLL      (1u << 3)
#define ONECAD_IOS_HAS_HOVER_Z   (1u << 4)
#define ONECAD_IOS_HAS_ESTIMATION_INDEX (1u << 5)
#define ONECAD_IOS_HAS_LOCATION  (1u << 6) /* clear only for some estimation updates */

typedef struct onecad_ios_touch_sample {
    uint64_t contact_id;         /* stable for one physical contact */
    uint32_t kind;               /* onecad_ios_contact_kind */
    uint32_t phase;              /* onecad_ios_phase */
    uint32_t provenance;         /* onecad_ios_provenance */
    uint32_t present;            /* ONECAD_IOS_HAS_* */
    uint32_t estimated;          /* ONECAD_IOS_HAS_* still estimated */
    uint32_t expects_update;     /* ONECAD_IOS_HAS_* expecting an update */
    double timestamp_s;          /* UITouch.timestamp (monotonic) */
    double x_pt, y_pt;           /* view logical points */
    double force, max_force;     /* when HAS_FORCE */
    double altitude_rad, azimuth_rad, roll_rad;
    double hover_z;              /* normalized 0..1, not millimeters */
    uint64_t estimation_index;
    uint32_t tap_count;          /* UITouch.tapCount */
    uint32_t reserved;
} onecad_ios_touch_sample;

typedef struct onecad_ios_input_batch {
    uint32_t struct_size;
    uint32_t count;              /* <= 4096 */
    const onecad_ios_touch_sample *samples;
    uint64_t view_generation;    /* bumps on every surface resize/rotation */
} onecad_ios_input_batch;

onecad_ios_status onecad_ios_input_submit(onecad_ios_app *app,
                                          const onecad_ios_input_batch *batch);

/* System gesture/interruption: cancels every live contact for authoring. */
onecad_ios_status onecad_ios_input_cancel_all(onecad_ios_app *app, uint32_t reason);
#define ONECAD_IOS_CANCEL_SYSTEM_GESTURE 0u
#define ONECAD_IOS_CANCEL_SCENE_INACTIVE 1u
#define ONECAD_IOS_CANCEL_ORIENTATION    2u
#define ONECAD_IOS_CANCEL_TEXT_FOCUS     3u

/* Two-finger navigation recognized natively (UIPinch/UIPan). Points/radians. */
typedef struct onecad_ios_navigation {
    uint32_t struct_size;
    uint32_t phase;              /* onecad_ios_phase BEGAN/MOVED/ENDED/CANCELLED */
    double anchor_x_pt, anchor_y_pt;
    double pan_dx_pt, pan_dy_pt; /* delta since previous event */
    double scale_delta;          /* multiplicative, 1.0 = none */
    double rotation_delta_rad;
} onecad_ios_navigation;

onecad_ios_status onecad_ios_navigation_submit(onecad_ios_app *app,
                                               const onecad_ios_navigation *nav);

/* ---- Pencil accessories and capabilities ------------------------------ */

typedef enum onecad_ios_pencil_action {
    ONECAD_IOS_PENCIL_BARREL_DOUBLE_TAP = 0,
    ONECAD_IOS_PENCIL_SQUEEZE = 1
} onecad_ios_pencil_action;

typedef enum onecad_ios_preferred_action { /* UIPencilPreferredAction */
    ONECAD_IOS_PREF_IGNORE = 0,
    ONECAD_IOS_PREF_SWITCH_ERASER = 1,
    ONECAD_IOS_PREF_SWITCH_PREVIOUS = 2,
    ONECAD_IOS_PREF_SHOW_COLOR_PALETTE = 3,
    ONECAD_IOS_PREF_SHOW_INK_ATTRIBUTES = 4,
    ONECAD_IOS_PREF_SHOW_CONTEXTUAL_PALETTE = 5,
    ONECAD_IOS_PREF_RUN_SYSTEM_SHORTCUT = 6
} onecad_ios_preferred_action;

typedef struct onecad_ios_pencil_event {
    uint32_t struct_size;
    uint32_t action;             /* onecad_ios_pencil_action */
    uint32_t preferred;          /* onecad_ios_preferred_action */
    uint32_t phase;              /* squeeze: BEGAN/MOVED/ENDED/CANCELLED; tap: ENDED */
    uint32_t present;            /* ONECAD_IOS_HAS_* for the hover pose */
    double x_pt, y_pt, hover_z, altitude_rad, azimuth_rad, roll_rad;
    double timestamp_s;
} onecad_ios_pencil_event;

onecad_ios_status onecad_ios_pencil_event_submit(onecad_ios_app *app,
                                                 const onecad_ios_pencil_event *ev);

#define ONECAD_IOS_CAP_PENCIL_CONNECTED (1u << 0) /* a Pencil contact was seen */
#define ONECAD_IOS_CAP_HOVER            (1u << 1)
#define ONECAD_IOS_CAP_PRESSURE         (1u << 2)
#define ONECAD_IOS_CAP_BARREL_TAP       (1u << 3)
#define ONECAD_IOS_CAP_SQUEEZE          (1u << 4)
#define ONECAD_IOS_CAP_ROLL             (1u << 5)
#define ONECAD_IOS_CAP_HAPTICS          (1u << 6)
#define ONECAD_IOS_CAP_HARDWARE_KEYBOARD (1u << 7)

typedef struct onecad_ios_capabilities {
    uint32_t struct_size;
    uint32_t flags;              /* ONECAD_IOS_CAP_* observed at runtime */
    uint32_t prefers_pencil_only_drawing; /* UIPencilInteraction.prefersPencilOnlyDrawing */
    uint32_t preferred_tap_action;        /* onecad_ios_preferred_action */
    uint32_t preferred_squeeze_action;
} onecad_ios_capabilities;

onecad_ios_status onecad_ios_capabilities_set(onecad_ios_app *app,
                                              const onecad_ios_capabilities *caps);

/* ---- Keyboard and text ------------------------------------------------- */

typedef enum onecad_ios_text_kind {
    ONECAD_IOS_TEXT_GENERAL = 0,
    ONECAD_IOS_TEXT_NUMERIC_EXPRESSION = 1 /* dimension/parameter field */
} onecad_ios_text_kind;

#define ONECAD_IOS_MOD_SHIFT   (1u << 0)
#define ONECAD_IOS_MOD_CONTROL (1u << 1)
#define ONECAD_IOS_MOD_OPTION  (1u << 2)
#define ONECAD_IOS_MOD_COMMAND (1u << 3)

/* Draft-only text protocol v1. Offsets: UTF-16 scalar boundaries. Snapshot
 * strings are borrowed until the next text_snapshot call. Missing mark: UINT32_MAX.
 * Every mutation requires current generation/revision and a larger sequence;
 * rejected operations never mutate. Read a fresh snapshot after every response. */
typedef struct onecad_ios_text_snapshot_v1 {
    uint32_t struct_size, version;
    uint64_t generation, revision, sequence, field_id;
    const char *text;
    uint32_t text_len, selection_start, selection_end, marked_start, marked_end, active;
    float field_x, field_y, field_w, field_h;
    float caret_x, caret_y, caret_w, caret_h;
    const char *label;
} onecad_ios_text_snapshot_v1;

enum onecad_ios_text_operation_kind {
    ONECAD_IOS_TEXT_REPLACE = 0, ONECAD_IOS_TEXT_SELECT = 1,
    ONECAD_IOS_TEXT_MARK = 2, ONECAD_IOS_TEXT_UNMARK = 3,
    ONECAD_IOS_TEXT_CANCEL = 4, ONECAD_IOS_TEXT_CONFIRM = 5,
    ONECAD_IOS_TEXT_TAB = 6, ONECAD_IOS_TEXT_BLUR = 7
};
typedef struct onecad_ios_text_operation_v1 {
    uint32_t struct_size, version;
    uint64_t generation, revision, sequence;
    uint32_t kind, start, end, selected_start, selected_end, text_len;
    const char *text; /* text_len bytes, not NUL terminated; NULL only for zero bytes */
} onecad_ios_text_operation_v1;
onecad_ios_status onecad_ios_text_snapshot(onecad_ios_app *, onecad_ios_text_snapshot_v1 *out);
onecad_ios_status onecad_ios_text_apply(onecad_ios_app *, const onecad_ios_text_operation_v1 *operation);

typedef struct onecad_ios_key_event {
    uint32_t struct_size;
    uint32_t hid_usage;          /* UIKeyboardHIDUsage */
    uint32_t modifiers;          /* ONECAD_IOS_MOD_* */
    uint32_t pressed;            /* 1 down, 0 up */
    uint32_t repeat;
} onecad_ios_key_event;

onecad_ios_status onecad_ios_key_submit(onecad_ios_app *app, const onecad_ios_key_event *ev);
/* Committed text (software keyboard insertText, Scribble, paste). UTF-8. */
onecad_ios_status onecad_ios_text_insert(onecad_ios_app *app, const char *utf8);
onecad_ios_status onecad_ios_text_delete_backward(onecad_ios_app *app);
/* Marked (IME) text; NULL or "" ends composition without committing. */
onecad_ios_status onecad_ios_text_set_marked(onecad_ios_app *app, const char *utf8);
/* Host-owned keyboard state: visible height in points (0 when hidden). */
onecad_ios_status onecad_ios_keyboard_frame(onecad_ios_app *app, float overlap_pt);

/* ---- Lifecycle --------------------------------------------------------- */

/* Recovery status: 0 pending, 1 completed (inspect per-document outcomes),
 * 2 expired, 3 cancelled. JSON is borrowed until the next snapshot call.
 * Each JSON result carries generation, documentId, runtimeSession, outcome and
 * durableOutcome. Expiry/cancellation never turns a late disk result into success.
 */
typedef struct onecad_ios_recovery_snapshot_result {
    uint32_t struct_size;
    uint32_t status;
    uint64_t generation;
    const char *documents_json;
} onecad_ios_recovery_snapshot_result;
onecad_ios_status onecad_ios_recovery_snapshot(onecad_ios_app *app,
    onecad_ios_recovery_snapshot_result *out_result);
/* reason: 0 system expiry; 1 foreground/superseded cancellation. Stale generations fail. */
onecad_ios_status onecad_ios_recovery_finish_waiting(onecad_ios_app *app,
    uint64_t generation, uint32_t reason);

typedef enum onecad_ios_lifecycle_event {
    ONECAD_IOS_WILL_RESIGN_ACTIVE = 0,
    ONECAD_IOS_DID_ENTER_BACKGROUND = 1,
    ONECAD_IOS_WILL_ENTER_FOREGROUND = 2,
    ONECAD_IOS_DID_BECOME_ACTIVE = 3,
    ONECAD_IOS_MEMORY_WARNING = 4,
    ONECAD_IOS_WILL_TERMINATE = 5,
    ONECAD_IOS_TRAIT_DARK = 6,   /* userInterfaceStyle changed to dark */
    ONECAD_IOS_TRAIT_LIGHT = 7
} onecad_ios_lifecycle_event;

onecad_ios_status onecad_ios_lifecycle(onecad_ios_app *app, uint32_t event);

/* ---- Accessibility ----------------------------------------------------- */

typedef enum onecad_ios_a11y_role {
    ONECAD_IOS_ROLE_OTHER = 0,
    ONECAD_IOS_ROLE_BUTTON = 1,
    ONECAD_IOS_ROLE_TEXT_FIELD = 2,
    ONECAD_IOS_ROLE_STATIC_TEXT = 3,
    ONECAD_IOS_ROLE_TAB = 4,
    ONECAD_IOS_ROLE_CHECKBOX = 5,
    ONECAD_IOS_ROLE_IMAGE = 6,
    ONECAD_IOS_ROLE_CANVAS = 7,
    ONECAD_IOS_ROLE_MENU_ITEM = 8,
    ONECAD_IOS_ROLE_SLIDER = 9,
    ONECAD_IOS_ROLE_GROUP = 10
} onecad_ios_a11y_role;

#define ONECAD_IOS_A11Y_DISABLED (1u << 0)
#define ONECAD_IOS_A11Y_SELECTED (1u << 1)
#define ONECAD_IOS_A11Y_FOCUSED  (1u << 2)
#define ONECAD_IOS_A11Y_HIDDEN   (1u << 3)

typedef struct onecad_ios_a11y_node {
    uint64_t id;                 /* stable while the widget exists */
    uint32_t role;               /* onecad_ios_a11y_role */
    uint32_t flags;              /* ONECAD_IOS_A11Y_* */
    float x, y, w, h;            /* logical points, view coordinates */
    const char *identifier;      /* stable automation id, never localized */
    const char *label;
    const char *value;
} onecad_ios_a11y_node;

/* Flattened, z-ordered list of actionable/visible nodes. */
onecad_ios_status onecad_ios_a11y_snapshot(onecad_ios_app *app,
                                           const onecad_ios_a11y_node **out_nodes,
                                           uint32_t *out_count, uint64_t *out_generation);
/* VoiceOver/automation activation of a node (equivalent to a tap). */
onecad_ios_status onecad_ios_a11y_activate(onecad_ios_app *app, uint64_t id);

/* ---- Host requests (document picker, export, share, haptics) ---------- */

typedef enum onecad_ios_host_request_kind {
    ONECAD_IOS_REQ_OPEN_DOCUMENT = 0, /* pick existing file(s); host copies into inbox */
    ONECAD_IOS_REQ_SAVE_DOCUMENT = 1, /* choose sandbox name for a new document */
    ONECAD_IOS_REQ_EXPORT_FILE = 2,   /* present export picker for a written file */
    ONECAD_IOS_REQ_HAPTIC = 3,
    ONECAD_IOS_REQ_COPY_TEXT = 4,
    ONECAD_IOS_REQ_OPEN_URL = 5
} onecad_ios_host_request_kind;

typedef struct onecad_ios_host_request {
    uint64_t id;
    uint32_t kind;               /* onecad_ios_host_request_kind */
    uint32_t multiple;
    const char *label;
    const char *extensions;      /* comma separated, without dots */
    const char *path;            /* EXPORT_FILE: sandbox file to publish */
    const char *suggested_name;
    const char *text;            /* COPY_TEXT / OPEN_URL / haptic style */
} onecad_ios_host_request;

/* Pops the next pending request; returns ONECAD_IOS_ERR_STATE when empty. */
onecad_ios_status onecad_ios_host_request_next(onecad_ios_app *app,
                                               onecad_ios_host_request *out);
/* Completes a request with zero or more sandbox paths (empty = cancelled). */
onecad_ios_status onecad_ios_host_request_complete(onecad_ios_app *app, uint64_t id,
                                                   const char *const *paths,
                                                   uint32_t count);
/* Opens a file the system handed to the app (Files "Open in", URL scheme) like
 * Open Recent; a .step/.stp file is imported into a new document instead. */
onecad_ios_status onecad_ios_open_path(onecad_ios_app *app, const char *utf8_path);

/* ---- Diagnostics ------------------------------------------------------- */

/* Last error message for this thread's most recent failing call. Valid until
 * the next call on the app. Never NULL. */
const char *onecad_ios_last_error(const onecad_ios_app *app);

#ifdef __cplusplus
}
#endif

#endif /* ONECAD_IOS_H */
