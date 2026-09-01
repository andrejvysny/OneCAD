/*
 * "Import components…" (Component Library WP-C2) — the batch call, the
 * per-row result render, and the refresh-on-`ok` contract. The BACKEND half
 * (a real STEP reader, a real worker) does not exist yet; this dialog's job
 * is the request/response wiring and the errors-as-values guarantee, both of
 * which are provable against a mocked `ingestComponents` alone.
 */
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { IngestComponentsDialog } from "./IngestComponentsDialog";

const ingestComponents = vi.fn();
const pickComponentFiles = vi.fn();
vi.mock("@/ipc/client", () => ({
  createClient: () => ({
    ingestComponents: (...args: unknown[]) => ingestComponents(...args),
    pickComponentFiles: (...args: unknown[]) => pickComponentFiles(...args),
  }),
}));

function fakeFile(name: string): File {
  return new File(["dummy"], name, { type: "application/octet-stream" });
}

beforeEach(() => {
  ingestComponents.mockReset();
  pickComponentFiles.mockReset();
  pickComponentFiles.mockResolvedValue([]); // mock-lane default: no native dialog
});

describe("IngestComponentsDialog", () => {
  it("renders nothing when closed", () => {
    render(
      <IngestComponentsDialog open={false} onClose={vi.fn()} onImported={vi.fn()} />,
    );
    expect(screen.queryByTestId("library-ingest-dialog")).toBeNull();
  });

  it("Import stays disabled until files are chosen", async () => {
    render(<IngestComponentsDialog open onClose={vi.fn()} onImported={vi.fn()} />);
    expect(screen.getByTestId("library-ingest-submit")).toBeDisabled();

    const input = screen.getByTestId("library-ingest-file-input") as HTMLInputElement;
    await userEvent.upload(input, [fakeFile("a.step")]);
    expect(screen.getByTestId("library-ingest-submit")).not.toBeDisabled();
  });

  it("sends the chosen files plus the vendor/category defaults, and renders one row per part", async () => {
    ingestComponents.mockResolvedValue({
      libraryRoot: "/lib",
      parts: [
        { path: "a.step", id: "vendor.a", version: "1.0.0", status: "ok" },
        { path: "b.step", status: "failed", message: "not a solid" },
      ],
    });
    const onImported = vi.fn();
    render(<IngestComponentsDialog open onClose={vi.fn()} onImported={onImported} />);

    const input = screen.getByTestId("library-ingest-file-input") as HTMLInputElement;
    await userEvent.upload(input, [fakeFile("a.step"), fakeFile("b.step")]);
    await userEvent.click(screen.getByTestId("library-ingest-submit"));

    await waitFor(() => expect(ingestComponents).toHaveBeenCalledTimes(1));
    expect(ingestComponents).toHaveBeenCalledWith({
      paths: ["a.step", "b.step"],
      defaults: { vendor: "vendor", category: ["imported"], tags: undefined },
    });

    const rows = await screen.findAllByTestId("library-ingest-result-row");
    expect(rows).toHaveLength(2);
    expect(rows[0]).toHaveAttribute("data-status", "ok");
    expect(rows[1]).toHaveAttribute("data-status", "failed");
    expect(rows[1]).toHaveTextContent("not a solid");

    // At least one `ok` part landed — the caller re-lists the catalog.
    expect(onImported).toHaveBeenCalledTimes(1);
  });

  it("does NOT refresh the catalog when every part refuses or fails", async () => {
    ingestComponents.mockResolvedValue({
      libraryRoot: "<mock>",
      parts: [{ path: "a.step", status: "refused", message: "MOCK LIMIT" }],
    });
    const onImported = vi.fn();
    render(<IngestComponentsDialog open onClose={vi.fn()} onImported={onImported} />);

    await userEvent.upload(
      screen.getByTestId("library-ingest-file-input") as HTMLInputElement,
      [fakeFile("a.step")],
    );
    await userEvent.click(screen.getByTestId("library-ingest-submit"));

    await screen.findAllByTestId("library-ingest-result-row");
    expect(onImported).not.toHaveBeenCalled();
  });

  it("a rejected round trip becomes a failed row per requested path, never a thrown error", async () => {
    ingestComponents.mockRejectedValue(new Error("no worker reachable"));
    render(<IngestComponentsDialog open onClose={vi.fn()} onImported={vi.fn()} />);

    await userEvent.upload(
      screen.getByTestId("library-ingest-file-input") as HTMLInputElement,
      [fakeFile("a.step"), fakeFile("b.step")],
    );
    // The click handler is async and must not throw past this call.
    await userEvent.click(screen.getByTestId("library-ingest-submit"));

    const rows = await screen.findAllByTestId("library-ingest-result-row");
    expect(rows).toHaveLength(2);
    for (const row of rows) {
      expect(row).toHaveAttribute("data-status", "failed");
      expect(row).toHaveTextContent("no worker reachable");
    }
  });

  it("resets its selection and result on re-open", async () => {
    ingestComponents.mockResolvedValue({
      libraryRoot: "/lib",
      parts: [{ path: "a.step", status: "ok" }],
    });
    const { rerender } = render(
      <IngestComponentsDialog open onClose={vi.fn()} onImported={vi.fn()} />,
    );
    await userEvent.upload(
      screen.getByTestId("library-ingest-file-input") as HTMLInputElement,
      [fakeFile("a.step")],
    );
    await userEvent.click(screen.getByTestId("library-ingest-submit"));
    await screen.findAllByTestId("library-ingest-result-row");

    rerender(<IngestComponentsDialog open={false} onClose={vi.fn()} onImported={vi.fn()} />);
    rerender(<IngestComponentsDialog open onClose={vi.fn()} onImported={vi.fn()} />);

    expect(screen.queryByTestId("library-ingest-result")).toBeNull();
    expect(screen.getByTestId("library-ingest-submit")).toBeDisabled();
  });
});

/** Real-lane picker (WP-C2): `__TAURI_INTERNALS__` present simulates the Tauri
 *  webview so `handleChoose`'s lane check takes the real branch. */
describe("IngestComponentsDialog — real-lane native picker (WP-C2)", () => {
  afterEach(() => {
    delete (window as unknown as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
  });

  it("uses pickComponentFiles's paths directly, skips the hidden input, and submits them verbatim", async () => {
    (window as unknown as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ = {};
    pickComponentFiles.mockResolvedValue(["/vendor/bracket.step", "/vendor/rail.stp"]);
    ingestComponents.mockResolvedValue({
      libraryRoot: "/lib",
      parts: [
        { path: "/vendor/bracket.step", status: "ok" },
        { path: "/vendor/rail.stp", status: "ok" },
      ],
    });

    render(<IngestComponentsDialog open onClose={vi.fn()} onImported={vi.fn()} />);

    await userEvent.click(screen.getByTestId("library-ingest-choose"));
    await waitFor(() => expect(pickComponentFiles).toHaveBeenCalledTimes(1));

    expect(await screen.findByText("bracket.step")).toBeInTheDocument();
    expect(screen.getByText("rail.stp")).toBeInTheDocument();
    expect(screen.getByTestId("library-ingest-submit")).not.toBeDisabled();

    await userEvent.click(screen.getByTestId("library-ingest-submit"));

    await waitFor(() => expect(ingestComponents).toHaveBeenCalledTimes(1));
    expect(ingestComponents).toHaveBeenCalledWith({
      paths: ["/vendor/bracket.step", "/vendor/rail.stp"],
      defaults: { vendor: "vendor", category: ["imported"], tags: undefined },
    });
  });

  it("treats an empty real-lane pick as a cancel — no fallback to the hidden input", async () => {
    (window as unknown as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ = {};
    pickComponentFiles.mockResolvedValue([]);
    render(<IngestComponentsDialog open onClose={vi.fn()} onImported={vi.fn()} />);

    await userEvent.click(screen.getByTestId("library-ingest-choose"));
    await waitFor(() => expect(pickComponentFiles).toHaveBeenCalledTimes(1));

    expect(screen.getByText("No files chosen")).toBeInTheDocument();
    expect(screen.getByTestId("library-ingest-submit")).toBeDisabled();
  });
});
