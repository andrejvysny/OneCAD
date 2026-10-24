import XCTest
@testable import OneCAD

@MainActor
final class FrameLoopTests: XCTestCase {
    func testWakeDuringFrameSurvivesAndSettledLoopIdles() {
        var frames = 0
        weak var pendingLoop: FrameLoop?
        let loop = FrameLoop {
            frames += 1
            if frames == 1 { pendingLoop?.requestFrame() }
        }
        pendingLoop = loop
        defer { loop.stop() }
        loop.tick()
        XCTAssertEqual(frames, 0)
        loop.requestFrame()
        loop.tick()
        XCTAssertEqual(frames, 1)
        XCTAssertTrue(loop.needsFrame)
        loop.tick()
        XCTAssertEqual(frames, 2)
        XCTAssertFalse(loop.needsFrame)
        loop.tick()
        XCTAssertEqual(frames, 2)
    }

    func testShortRetryUsesRefreshAndSuspensionRetainsOnlyPendingWork() {
        var frames = 0
        let loop = FrameLoop { frames += 1 }
        defer { loop.stop() }
        loop.scheduleRepaint(after: 0.016)
        XCTAssertTrue(loop.needsFrame)
        loop.setSuspended(true)
        loop.tick()
        XCTAssertEqual(frames, 0)
        XCTAssertTrue(loop.needsFrame)
        loop.setSuspended(false)
        loop.tick()
        XCTAssertEqual(frames, 1)
        XCTAssertFalse(loop.needsFrame)
        loop.scheduleRepaint(after: 60)
        XCTAssertFalse(loop.needsFrame)
        loop.requestFrame()
        loop.scheduleRepaint(after: 60)
        XCTAssertTrue(loop.needsFrame)
        loop.stop()
        XCTAssertFalse(loop.needsFrame)
    }

    func testNonfiniteAndDisabledDelaysNeverCreateImmediateWork() {
        var frames = 0
        let loop = FrameLoop { frames += 1 }
        defer { loop.stop() }
        for delay in [Double.nan, .infinity, -.infinity, -1] {
            loop.scheduleRepaint(after: delay)
            XCTAssertFalse(loop.needsFrame)
            loop.tick()
        }
        XCTAssertEqual(frames, 0)
        loop.scheduleRepaint(after: 0)
        loop.tick()
        XCTAssertEqual(frames, 1)
    }
}
