// link_probe.cpp — references the in-process worker's C ABI the way a host
// does, so linking it pulls exactly the objects a host link pulls. Built by
// scripts/build-ios-worker.sh (target onecad_worker_link_probe) to derive the
// OCCT toolkits the embedded bundle needs; never run, never shipped.
#include "capi/onecad_worker.h"

int main() {
    // Taking the addresses keeps both run entry points referenced.
    const bool linked = onecad_worker_run != nullptr && onecad_worker_run_config != nullptr;
    return linked && onecad_worker_abi_version() == ONECAD_WORKER_ABI_VERSION ? 0 : 1;
}
