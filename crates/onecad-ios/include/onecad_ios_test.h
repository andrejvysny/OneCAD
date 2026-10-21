/* onecad_ios_test.h — acceptance-only test hooks.
 *
 * Exported only when the Rust crate is built with the nondefault `test-hooks`
 * feature (Xcode configuration `Acceptance`). Debug and Release products must
 * not contain any onecad_ios_test_ symbol; `cargo xtask ios-simulator-gate`
 * checks the shipped archive with nm. Main thread only, non-blocking.
 *
 * Commands are JSON objects {"cmd": "...", ...}; responses are JSON objects
 * {"ok": true, ...} or {"ok": false, "error": "..."}. See docs/ios/TESTING.md
 * for the command catalogue. Synthetic Pencil traces submitted here go through
 * the same normalization and routing as onecad_ios_input_submit and are
 * labeled synthetic in every trace record they produce.
 */
#ifndef ONECAD_IOS_TEST_H
#define ONECAD_IOS_TEST_H

#include "onecad_ios.h"

#ifdef __cplusplus
extern "C" {
#endif

onecad_ios_status onecad_ios_test_command(onecad_ios_app *app, const char *json_utf8,
                                          char **out_json_utf8);
void onecad_ios_test_string_free(char *s);

#ifdef __cplusplus
}
#endif

#endif /* ONECAD_IOS_TEST_H */
