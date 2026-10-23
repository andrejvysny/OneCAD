import UIKit
import XCTest
@testable import OneCAD

@MainActor
final class SaveNamePromptNamingTests: XCTestCase {
    func testEverySupportedFormatNormalizesExactlyOneSuffix() throws {
        for fileExtension in ["onecad", "step", "stl", "obj", "3mf"] {
            let name = "Named plate.\(fileExtension.uppercased()).\(fileExtension)"
            let url = try XCTUnwrap(SaveNamePrompt.destination(for: name, fileExtension: fileExtension))
            XCTAssertEqual(url.lastPathComponent, "Named plate.\(fileExtension)")
            XCTAssertEqual(SaveNamePrompt.baseName(name, fileExtension: fileExtension), "Named plate")
        }
    }

    func testInvalidNamesAndUnsupportedFormatsAreRefusedAndSeparatorsStayConfined() throws {
        for name in ["", "   ", ".", "..", ". .", ".step.STEP", "bad\0name", "bad\nname"] {
            XCTAssertNil(SaveNamePrompt.destination(for: name, fileExtension: "step"), name)
        }
        for fileExtension in ["", "exe", "../step", "step,stl"] {
            XCTAssertNil(SaveNamePrompt.destination(for: "plate", fileExtension: fileExtension))
        }
        let url = try XCTUnwrap(SaveNamePrompt.destination(for: "../plate\\part:copy.STEP",
                                                         fileExtension: "step"))
        let documents = try FileManager.default.url(for: .documentDirectory, in: .userDomainMask,
                                                    appropriateFor: nil, create: true)
        XCTAssertEqual(url.deletingLastPathComponent().standardizedFileURL, documents.standardizedFileURL)
        XCTAssertEqual(url.lastPathComponent, "-plate-part-copy.step")
    }
}

#if ONECAD_ACCEPTANCE
/// Copies leave the sandbox only through the system export picker (managed
/// copy policy, docs/ios/HOST.md): File › Export STEP and File › Send a Copy
/// reach it from the editor's own menu rows. The STEP name prompt is answered
/// by `answer_save`; the picker itself is real and is cancelled here.
@MainActor
final class DocumentExportTests: XCTestCase {
    private var driver: DocumentDriver?
    private var files: [URL] = []

    override func tearDown() async throws {
        if let picker = Self.presentedPicker() { Self.cancel(picker) }
        try? await driver?.closeEveryTab()
        for url in files { try? FileManager.default.removeItem(at: url) }
    }

    private func start() async throws -> DocumentDriver {
        let docs = try await DocumentDriver.attach(self)
        driver = docs
        try await docs.closeEveryTab()
        return docs
    }

    func testStepExportReachesTheExportPicker() async throws {
        let docs = try await start()
        _ = try await docs.openMacPlate(as: "export-plate")
        let name = "export-\(UUID().uuidString.prefix(8)).step"
        let answer = try docs.hooks.call(["cmd": "answer_save", "name": name])
        let step = URL(fileURLWithPath: try XCTUnwrap(answer["path"] as? String))
        files.append(step)
        try await docs.file("onecad.menu.file.export")
        let picker = try await waitForPicker()
        let header = try String(decoding: Data(contentsOf: step).prefix(13), as: UTF8.self)
        XCTAssertEqual(header, "ISO-10303-21;", "the kernel wrote a STEP file before the picker opened")
        Self.cancel(picker)
        let after = try await docs.waitForSummary("export reported") { !DocumentDriver.busy($0) }
        XCTAssertTrue(DocumentDriver.status(after).hasPrefix("Exported "), DocumentDriver.status(after))
        XCTAssertEqual(DocumentDriver.dirty(after), false, "exporting never edits the document")
    }

    /// The saved `.onecad` goes to the picker; with unsaved changes nothing
    /// is sent and the status says why.
    func testSendACopyOffersOnlyTheSavedDocument() async throws {
        let docs = try await start()
        let plate = try await docs.openMacPlate(as: "copy-plate")
        XCTAssertNotNil(DocumentDriver.document(plate)["path"] as? String)
        try await docs.file("onecad.menu.file.sendCopy")
        let picker = try await waitForPicker()
        let sent = try await docs.waitForSummary("copy offered") { !DocumentDriver.busy($0) }
        XCTAssertEqual(DocumentDriver.status(sent), "Choose where to send a copy")
        Self.cancel(picker)
        try await waitFor("picker dismissed", timeout: 15) { Self.presentedPicker() == nil }
        try await docs.addSketch()
        try await docs.file("onecad.menu.file.sendCopy")
        let refused = try await docs.waitForSummary("copy refused") {
            DocumentDriver.status($0) == "Save your changes before sending a copy"
        }
        XCTAssertEqual(DocumentDriver.dirty(refused), true)
        XCTAssertNil(Self.presentedPicker(), "a stale copy must not be offered")
    }

    private func waitForPicker() async throws -> UIDocumentPickerViewController {
        try await waitFor("export picker presented", timeout: 30) { Self.presentedPicker() != nil }
        return try XCTUnwrap(Self.presentedPicker())
    }

    static func presentedPicker() -> UIDocumentPickerViewController? {
        let windows = UIApplication.shared.connectedScenes
            .compactMap { $0 as? UIWindowScene }
            .flatMap(\.windows)
        for window in windows {
            var controller = window.rootViewController
            while let next = controller?.presentedViewController {
                if let picker = next as? UIDocumentPickerViewController { return picker }
                controller = next
            }
        }
        return nil
    }

    /// What the picker's Cancel button does: the host completes the request
    /// and dismisses it.
    static func cancel(_ picker: UIDocumentPickerViewController) {
        picker.delegate?.documentPickerWasCancelled?(picker)
        picker.dismiss(animated: false)
    }
}
#else
/// Makes a non-Acceptance run visibly skip instead of passing with 0 tests.
final class DocumentExportTests: XCTestCase {
    func testStepExportReachesTheExportPicker() throws {
        throw XCTSkip("semantic test hooks exist only in the Acceptance configuration")
    }

    func testSendACopyOffersOnlyTheSavedDocument() throws {
        throw XCTSkip("semantic test hooks exist only in the Acceptance configuration")
    }
}
#endif
