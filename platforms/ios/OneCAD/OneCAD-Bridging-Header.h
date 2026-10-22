// Imports the frozen C ABI (crates/onecad-ios/include) into the app module.
#include "onecad_ios.h"

// Acceptance-only hooks: the Release and Debug Rust archives do not export
// them, so their declarations stay out of those builds as well.
#if defined(ONECAD_ACCEPTANCE)
#include "onecad_ios_test.h"
#endif
