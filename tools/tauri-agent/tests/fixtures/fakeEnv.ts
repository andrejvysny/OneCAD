/**
 * Fakes for the acting tools. NOTHING here posts real input: every native call is recorded
 * in `order` and `calls` so a test can assert the pipeline's sequence without a window
 * server, a TCC grant, or a running app.
 */
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import type { CallToolResult } from "@modelcontextprotocol/sdk/types.js";
import { AgentError } from "../../src/errors.ts";
import type { Pt, Rect, WindowGeom } from "../../src/geometry/types.ts";
import type {
  CaptureResult,
  MouseButton,
  NativeAccessibility,
  NativeInput,
  NativeWindowInfo,
  PlatformAdapter,
} from "../../src/platform/adapter.ts";
import type { ExecuteOpts } from "../../src/semantic/webdriver.ts";
import type { IdleReading } from "../../src/semantic/idleScript.ts";
import type { SnapNode } from "../../src/semantic/snapshot.ts";
import type { AxRefStore } from "../../src/native/resolve.ts";
import type { AgentConfig, ResolvedConfig } from "../../src/session/config.ts";
import { parseConfig } from "../../src/session/config.ts";
import type { CaptureCapability, InteractionPolicy, SessionBridge } from "../../src/session/types.ts";
import { Journal } from "../../src/trace/journal.ts";
import type { ToolCtx } from "../../src/mcp/defineTool.ts";
import type { ActionResult } from "../../src/mcp/envelope.ts";
import type { PipelineSession, WheelProbeLike } from "../../src/mcp/tools/actionPipeline.ts";
import { registerAllTools } from "../../src/mcp/tools/index.ts";
import { unusedAx } from "./fakeAx.ts";

export const GEOM: WindowGeom = {
  innerPositionPx: { x: 200, y: 100 },
  innerSizePx: { width: 1600, height: 1000 },
  scaleFactor: 2,
  nativeBoundsPt: { x: 100, y: 50, width: 800, height: 500 },
  windowId: 77,
};

export const RECT: Rect = { x: 40, y: 60, width: 100, height: 20 };

export function fakeNode(over: Partial<SnapNode> & { ref: string }): SnapNode {
  return {
    fp: `fp-${over.ref}`,
    role: "button",
    name: "Extrude",
    rect: RECT,
    depth: 0,
    state: {},
    css: "#extrude",
    dragRegion: false,
    disabled: false,
    ...over,
  };
}

export class FakeBridge implements SessionBridge {
  readonly wedged = false;
  readonly scripts: string[] = [];
  readonly invoked: string[] = [];
  rev = 1;
  consoleErrors = 0;
  hitIsSelf = true;
  bodyText = "";
  /** Default settle-tuple reading: everything published, nothing busy, nothing moving. */
  idle: Omit<IdleReading, "rev"> = {
    regenBusy: 0,
    geometryPending: false,
    documentRevision: 1,
    frames: 1,
    camera: null,
  };
  canned: Record<string, unknown> = {};

  constructor(private readonly order: string[]) {}

  async execute<T>(name: string, _fn: unknown, args: unknown[], _opts: ExecuteOpts): Promise<T> {
    this.scripts.push(name);
    if (Object.hasOwn(this.canned, name)) {
      const v = this.canned[name];
      return (typeof v === "function" ? (v as (a: unknown[]) => unknown)(args) : v) as T;
    }
    switch (name) {
      case "ui_revision":
        this.order.push("revision");
        return { rev: this.rev, lastMutationAt: 0, now: 0 } as T;
      case "ui_idle":
        this.order.push("idle");
        return { rev: this.rev, ...this.idle } as T;
      case "effects.consoleErrors":
        return this.consoleErrors as T;
      case "resolve.forInput":
        this.order.push("resolve");
        return {
          found: true,
          fp: (args[0] as string | null) === null ? null : `fp-${String(args[0])}`,
          fpAvailable: true,
          rect0: RECT,
          rect1: RECT,
          dragRegion: false,
          hitIsSelf: true,
          occluder: null,
        } as T;
      case "pointer.recheckHit":
        this.order.push("recheck");
        return this.hitIsSelf as T;
      case "wait.text":
        return this.bodyText.includes(String(args[0])) as T;
      default:
        throw new Error(`unexpected bridged script: ${name}`);
    }
  }

  /** Mirrors the real bridge, which names an invoke script `invoke:<command>`. */
  async invoke<T>(command: string): Promise<T> {
    this.invoked.push(command);
    const key = `invoke:${command}`;
    if (Object.hasOwn(this.canned, key)) {
      const v = this.canned[key];
      return (typeof v === "function" ? (v as () => unknown)() : v) as T;
    }
    return null as T;
  }

  async windowGeom(): Promise<never> {
    throw new Error("not used");
  }

  async close(): Promise<void> {}
}

export interface InputCall {
  verb: string;
  button?: MouseButton;
  point?: Pt;
  opts?: unknown;
}

export class FakeInput implements NativeInput {
  readonly calls: InputCall[] = [];
  failOn?: string;

  constructor(private readonly order: string[]) {}

  #record(verb: string, rest: Omit<InputCall, "verb">): void {
    this.order.push(verb);
    this.calls.push({ verb, ...rest });
    if (this.failOn === verb) throw new AgentError("INTERNAL", `fake input failed on ${verb}`);
  }

  async move(p: Pt, opts?: unknown): Promise<void> {
    this.#record("move", { point: p, opts });
  }
  async down(button: MouseButton, p: Pt, opts?: unknown): Promise<void> {
    this.#record("down", { button, point: p, opts });
  }
  async up(button: MouseButton, p: Pt, opts?: unknown): Promise<void> {
    this.#record("up", { button, point: p, opts });
  }
  async click(button: MouseButton, p: Pt, opts?: unknown): Promise<void> {
    this.#record("click", { button, point: p, opts });
  }
  async path(button: MouseButton, points: Pt[], opts?: unknown): Promise<void> {
    this.#record("path", { button, opts: { points, ...(opts as object) } });
  }
  async scroll(p: Pt, delta: unknown, opts?: unknown): Promise<void> {
    this.#record("scroll", { point: p, opts: { delta, ...(opts as object) } });
  }
  async keyDown(key: string, opts?: unknown): Promise<void> {
    this.#record("keyDown", { opts: { key, ...(opts as object) } });
  }
  async keyUp(key: string, opts?: unknown): Promise<void> {
    this.#record("keyUp", { opts: { key, ...(opts as object) } });
  }
  async press(key: string, mods?: unknown): Promise<void> {
    this.#record("press", { opts: { key, mods } });
  }
  async type(text: string, opts?: unknown): Promise<void> {
    this.#record("type", { opts: { text, ...(opts as object) } });
  }
  async releaseAll(): Promise<void> {
    this.order.push("releaseAll");
    this.calls.push({ verb: "releaseAll" });
  }
  async cursor(): Promise<Pt> {
    return { x: 500, y: 300 };
  }
  async permissions(): Promise<{ accessibility: boolean; screenRecording: boolean }> {
    return { accessibility: true, screenRecording: true };
  }
  async dispose(): Promise<void> {}
}

export function fakePlatform(
  order: string[],
  windows: NativeWindowInfo[] = [],
  ax: NativeAccessibility = unusedAx(),
): PlatformAdapter & { input: FakeInput } {
  const input = new FakeInput(order);
  const shot = (outPath: string): CaptureResult => {
    order.push("capture");
    writeFileSync(outPath, Buffer.from([0x89, 0x50, 0x4e, 0x47]));
    return { width: 100, height: 60, pixelScale: 2 };
  };
  return {
    name: "macos",
    input,
    ax,
    windows: {
      list: async () => windows,
      isFrontmost: async () => true,
      focus: async () => true,
    },
    capture: {
      window: async (_id, outPath) => shot(outPath),
      region: async (_r, outPath) => shot(outPath),
      screen: async (outPath) => shot(outPath),
      preview: async (_src, outPath) => {
        writeFileSync(outPath, Buffer.from([0x89, 0x50, 0x4e, 0x47]));
      },
    },
  };
}

export class FakeSession implements PipelineSession {
  readonly refs = new Map<string, SnapNode>();
  /** The accessibility pool, written by the native tools and by the native settle. */
  axRefs?: AxRefStore;
  wheelLinesPerNotch = 3;
  windowId = GEOM.windowId;
  frontmost = true;
  /** Which policy the fake session reports; `defineTool` stamps it on every envelope. */
  interactionPolicy: InteractionPolicy = "foreground";
  moved = false;
  reconnects = 0;
  wheelProbe: WheelProbeLike = { ok: true, device: "mouse", linesPerNotch: 3 };
  pid: number | undefined = 4242;
  launched = true;
  /** Flip to a degraded session to exercise the no-Screen-Recording paths. */
  capture: CaptureCapability = { available: true, authoritative: true };

  constructor(
    readonly order: string[],
    readonly bridge: FakeBridge,
    readonly platform: PlatformAdapter,
  ) {}

  requireReady(): void {
    this.order.push("ready");
  }
  async ensureCalibrated(): Promise<void> {
    this.order.push("calibrate");
  }
  async assertFrontmost(): Promise<void> {
    this.order.push("frontmost");
    if (!this.frontmost) throw new AgentError("WINDOW_NOT_FOREGROUND", "fake app is not frontmost");
  }

  async assertFrontmostOrFocus(): Promise<boolean> {
    this.order.push("focus");
    // Models a window that cannot be brought forward: the real method throws the same code when
    // both the frontmost check and the focus attempt fail.
    if (!this.frontmost) throw new AgentError("WINDOW_NOT_FOREGROUND", "fake app cannot be focused");
    // `activated` mirrors the real method: true only when it had to bring the app forward.
    return this.activatesOnFocus;
  }

  /** Set true to model an app that WAS behind and had to be activated by the gate. */
  activatesOnFocus = false;
  requireBridge(): SessionBridge {
    return this.bridge;
  }
  /**
   * Set to hand the tools a DIFFERENT adapter than the one the fake records against — the point
   * being to install the real refusing wrapper over the recording fake, so a test can drive the
   * shipped tools against the seam and still read `input.calls` to prove nothing got through.
   */
  platformOverride?: PlatformAdapter;

  requirePlatform(): PlatformAdapter {
    return this.platformOverride ?? this.platform;
  }
  requireGeom(): WindowGeom {
    return GEOM;
  }
  async geometryChanged(): Promise<boolean> {
    return this.moved;
  }
  async reconnectBridge(): Promise<void> {
    this.reconnects += 1;
  }
  async ensureWheelProbe(): Promise<WheelProbeLike> {
    this.order.push("wheelProbe");
    return this.wheelProbe;
  }
  status(): { pid?: number; launched: boolean; capture: CaptureCapability } {
    return {
      ...(this.pid === undefined ? {} : { pid: this.pid }),
      launched: this.launched,
      capture: this.capture,
    };
  }
}

export interface Harness {
  order: string[];
  bridge: FakeBridge;
  session: FakeSession;
  input: FakeInput;
  journal: Journal;
  config: ResolvedConfig;
  ctx: ToolCtx;
  call(name: string, args: Record<string, unknown>): Promise<ActionResult>;
  raw(name: string, args: Record<string, unknown>): Promise<CallToolResult>;
  names(): string[];
}

type Registered = (args: unknown) => Promise<CallToolResult>;

/**
 * Registers the real tools against a stub McpServer so tests drive the shipped handlers.
 *
 * `native` supplies the accessibility side: the windows this app owns (what `checkGlobalPoint`
 * gates against) and a canned `NativeAccessibility`. Omitted, accessibility throws if touched,
 * which is what keeps a webview test honest about never reaching it.
 */
export function harness(
  overrides: Partial<AgentConfig> = {},
  native: { windows?: NativeWindowInfo[]; ax?: NativeAccessibility } = {},
): Harness {
  const order: string[] = [];
  const bridge = new FakeBridge(order);
  const platform = fakePlatform(order, native.windows ?? [], native.ax ?? unusedAx());
  const session = new FakeSession(order, bridge, platform);
  const dir = mkdtempSync(join(tmpdir(), "tauri-agent-pipeline-"));
  const journal = new Journal(join(dir, "session"));
  const config: ResolvedConfig = {
    root: dir,
    configPath: join(dir, "tauri-agent.config.json"),
    config: {
      ...parseConfig({ settle: { frameMs: 1, quietMs: 1, timeoutMs: 40 }, input: { dwellMs: 1, clickIntervalMs: 1 } }, "test"),
      ...overrides,
    },
  };
  const ctx = { session, journal, config } as unknown as ToolCtx;
  const tools = new Map<string, Registered>();
  const server = {
    registerTool(name: string, _cfg: unknown, cb: Registered) {
      tools.set(name, cb);
    },
  } as unknown as McpServer;
  registerAllTools(server, ctx);

  const raw = async (name: string, args: Record<string, unknown>): Promise<CallToolResult> => {
    const tool = tools.get(name);
    if (!tool) throw new Error(`tool not registered: ${name}`);
    return tool(args);
  };
  return {
    order,
    bridge,
    session,
    input: platform.input,
    journal,
    config,
    ctx,
    raw,
    names: () => [...tools.keys()],
    call: async (name, args) => {
      const result = await raw(name, args);
      const text = (result.content[0] as { text: string }).text;
      return JSON.parse(text) as ActionResult;
    },
  };
}
