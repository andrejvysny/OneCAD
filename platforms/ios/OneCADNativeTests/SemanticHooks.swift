import XCTest
@testable import OneCAD

#if ONECAD_ACCEPTANCE
/// Swift wrapper of the semantic acceptance commands of
/// `onecad_ios_test_command` (catalogue: docs/ios/TESTING.md). Commands that
/// wait on document services answer with a ticket; `run` polls it on the main
/// run loop while the display link keeps drawing frames.
@MainActor
struct SemanticHooks {
    let app: OpaquePointer

    struct Failure: Error, CustomStringConvertible {
        let description: String
    }

    /// The process's running bridge, waiting for the scene to create it.
    static func attach(_ test: XCTestCase, timeout: TimeInterval = 20) async throws -> SemanticHooks {
        try await test.waitFor("Rust bridge running", timeout: timeout) { AppDelegate.shared?.bridge?.app != nil }
        let app = try XCTUnwrap(AppDelegate.shared?.bridge?.app, "hosted app has no running Rust bridge")
        return SemanticHooks(app: app)
    }

    /// One synchronous command; `ok:false` becomes a thrown `Failure`.
    func call(_ request: [String: Any]) throws -> [String: Any] {
        let data = try JSONSerialization.data(withJSONObject: request)
        var response: UnsafeMutablePointer<CChar>?
        let status = String(decoding: data, as: UTF8.self).withCString {
            onecad_ios_test_command(app, $0, &response)
        }
        defer { onecad_ios_test_string_free(response) }
        guard status == ONECAD_IOS_OK, let response else {
            throw Failure(description: "onecad_ios_test_command status \(status.rawValue): \(String(cString: onecad_ios_last_error(app)))")
        }
        let text = String(cString: response)
        let object = try JSONSerialization.jsonObject(with: Data(text.utf8)) as? [String: Any] ?? [:]
        guard object["ok"] as? Bool == true else { throw Failure(description: text) }
        return object
    }

    /// A command to completion: synchronous ones return at once, ticketed
    /// ones are polled with `result` until they finish or `timeout` passes.
    func run(_ request: [String: Any], in test: XCTestCase, timeout: TimeInterval = 60) async throws -> [String: Any] {
        let reply = MainSlot(try call(request))
        guard reply.value["pending"] as? Bool == true, let ticket = reply.value["ticket"] as? Int else {
            return reply.value
        }
        // A failed result throws out of the poll (waitFor rethrows it).
        try await test.waitFor("\(request["cmd"] ?? "command") result", timeout: timeout) {
            reply.value = try call(["cmd": "result", "ticket": ticket])
            return reply.value["pending"] as? Bool != true
        }
        return reply.value
    }

    var runID: String? { (try? call(["cmd": "run_id"]))?["runId"] as? String }

    func kernelIdentity() throws -> [String: Any] {
        try call(["cmd": "kernel_identity"])["kernelIdentity"] as? [String: Any] ?? [:]
    }

    func documentSummary(in test: XCTestCase) async throws -> [String: Any] {
        try await run(["cmd": "document_summary"], in: test)["summary"] as? [String: Any] ?? [:]
    }

    func saveArtifact(_ name: String, in test: XCTestCase) async throws -> [String: Any] {
        try await run(["cmd": "save_artifact", "name": name], in: test)
    }

    func exportStepArtifact(_ name: String, in test: XCTestCase) async throws -> [String: Any] {
        try await run(["cmd": "export_step_artifact", "name": name], in: test)
    }

    func writeSemantic(_ name: String, in test: XCTestCase) async throws -> [String: Any] {
        try await run(["cmd": "write_semantic", "name": name], in: test)
    }

    /// `journal_write`, `save_failure` or `clear`.
    @discardableResult
    func fault(_ kind: String) throws -> [String: Any] {
        try call(["cmd": "fault", "kind": kind])
    }
}

extension XCTestCase {
    /// Yields to the main run loop for one poll interval (frames keep running).
    @MainActor
    func nextRunLoopTick() async throws {
        let polled = MainSlot(false)
        try await waitFor("next run loop tick", timeout: 5) {
            defer { polled.value = true }
            return polled.value
        }
    }

    /// Polls `condition` on the main run loop (never a sleep) until it holds;
    /// throws on timeout.
    @MainActor
    func waitFor(_ what: String, timeout: TimeInterval, _ condition: @escaping @MainActor () throws -> Bool) async throws {
        if try condition() { return }
        let done = expectation(description: what)
        done.assertForOverFulfill = false
        let thrown = MainSlot<Error?>(nil)
        // The timer itself is not Sendable: it stops checking once finished and
        // is invalidated below, after the wait.
        let finished = MainSlot(false)
        let poll = Timer.scheduledTimer(withTimeInterval: 0.05, repeats: true) { _ in
            MainActor.assumeIsolated {
                guard !finished.value else { return }
                do {
                    guard try condition() else { return }
                } catch {
                    thrown.value = error
                }
                finished.value = true
                done.fulfill()
            }
        }
        let result = await XCTWaiter().fulfillment(of: [done], timeout: timeout)
        poll.invalidate()
        if let error = thrown.value { throw error }
        guard result == .completed else { throw SemanticHooks.Failure(description: "timed out after \(timeout)s: \(what)") }
    }
}

/// Main-actor state shared with polling closures (Swift 6 forbids mutating
/// captured variables from them).
@MainActor
final class MainSlot<Value> {
    var value: Value
    init(_ value: Value) { self.value = value }
}
#endif
