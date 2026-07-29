import type { FunctorClient } from "./game.js";
import type { ModelJson, MouseButtonName, RuntimeState } from "./types.js";

/** The serializable plan shared with `functor mcp`'s restricted source dialect. */
export const AUTOMATION_PLAN_VERSION = 1 as const;
export const AUTOMATION_DEFAULT_STEP_DT = 0.016;
export const AUTOMATION_MAX_SOURCE_BYTES = 16 * 1024;
export const AUTOMATION_MAX_STEPS = 64;
export const AUTOMATION_MAX_TOTAL_FRAMES = 10_000;
export const AUTOMATION_MAX_CAPTURES = 4;
export const AUTOMATION_MAX_LITERAL_DEPTH = 8;

export type AutomationStep =
  | { type: "pause"; tts?: number }
  | { type: "key"; key: string; down: boolean }
  | { type: "press_key"; key: string }
  | { type: "mouse_move"; x: number; y: number }
  | { type: "mouse_button"; button: MouseButtonName; down: boolean }
  | { type: "mouse_wheel"; delta: number }
  | { type: "ui_click"; slot: number }
  | { type: "step"; frames: number; dts: number }
  | { type: "inspect"; label?: string }
  | { type: "expect_model"; path: string; equals: ModelJson }
  | {
      type: "expect_model_close";
      path: string;
      expected: number;
      abs_tolerance: number;
    }
  | { type: "capture"; label?: string };

export interface AutomationPlan {
  version: typeof AUTOMATION_PLAN_VERSION;
  name?: string;
  steps: AutomationStep[];
}

export interface AutomationStepOptions {
  frames?: number;
  dts?: number;
}

export interface AutomationObservation {
  label?: string;
  state: RuntimeState;
}

export interface AutomationAssertion {
  path: string;
  expected: ModelJson;
  actual: ModelJson;
  absTolerance?: number;
  passed: true;
}

export interface AutomationCapture {
  label?: string;
  png: Buffer;
}

export interface AutomationRunResult {
  plan: AutomationPlan;
  assertions: AutomationAssertion[];
  observations: AutomationObservation[];
  captures: AutomationCapture[];
  finalState: RuntimeState;
}

/** The existing typed SDK operations needed by the declarative plan executor. */
export type AutomationClient = Pick<
  FunctorClient,
  | "pause"
  | "keyDown"
  | "keyUp"
  | "mouseMove"
  | "mouseDown"
  | "mouseUp"
  | "mouseWheel"
  | "input"
  | "stepFrames"
  | "state"
  | "capture"
>;

/**
 * The fluent builder vocabulary shared by standalone SDK programs and the
 * restricted source accepted by `functor mcp`.
 *
 * Standalone TypeScript remains ordinary trusted TypeScript: use variables,
 * loops, and callbacks around this builder if useful. MCP-submitted source is
 * deliberately narrower and accepts only one literal `automation(...).…`
 * chain; the server parses that chain and never evaluates JavaScript.
 */
export class AutomationBuilder {
  readonly #name: string | undefined;
  readonly #steps: AutomationStep[] = [];

  constructor(name?: string) {
    this.#name = name;
  }

  pause(tts?: number): this {
    this.#steps.push(tts === undefined ? { type: "pause" } : { type: "pause", tts });
    return this;
  }

  keyDown(key: string): this {
    this.#steps.push({ type: "key", key, down: true });
    return this;
  }

  keyUp(key: string): this {
    this.#steps.push({ type: "key", key, down: false });
    return this;
  }

  /** Press for one deterministic 16ms step, then release even if stepping fails. */
  pressKey(key: string): this {
    this.#steps.push({ type: "press_key", key });
    return this;
  }

  mouseMove(x: number, y: number): this {
    this.#steps.push({ type: "mouse_move", x, y });
    return this;
  }

  mouseDown(button: MouseButtonName = "left"): this {
    this.#steps.push({ type: "mouse_button", button, down: true });
    return this;
  }

  mouseUp(button: MouseButtonName = "left"): this {
    this.#steps.push({ type: "mouse_button", button, down: false });
    return this;
  }

  mouseWheel(delta: number): this {
    this.#steps.push({ type: "mouse_wheel", delta });
    return this;
  }

  uiClick(slot: number): this {
    this.#steps.push({ type: "ui_click", slot });
    return this;
  }

  step(options: AutomationStepOptions = {}): this {
    this.#steps.push({
      type: "step",
      frames: options.frames ?? 1,
      dts: options.dts ?? AUTOMATION_DEFAULT_STEP_DT,
    });
    return this;
  }

  inspect(label?: string): this {
    this.#steps.push(
      label === undefined ? { type: "inspect" } : { type: "inspect", label },
    );
    return this;
  }

  expectModel(path: string, equals: ModelJson): this {
    this.#steps.push({ type: "expect_model", path, equals });
    return this;
  }

  expectModelClose(path: string, expected: number, absTolerance: number): this {
    this.#steps.push({
      type: "expect_model_close",
      path,
      expected,
      abs_tolerance: absTolerance,
    });
    return this;
  }

  capture(label?: string): this {
    this.#steps.push(
      label === undefined ? { type: "capture" } : { type: "capture", label },
    );
    return this;
  }

  /** Return a detached plain-data plan suitable for logging or serialization. */
  toPlan(): AutomationPlan {
    const plan: AutomationPlan = {
      version: AUTOMATION_PLAN_VERSION,
      steps: this.#steps,
    };
    if (this.#name !== undefined) plan.name = this.#name;
    return structuredClone(plan);
  }

  /** Canonical restricted source that can be submitted to `functor mcp`. */
  toCode(): string {
    return canonicalAutomationCode(this);
  }
}

export function automation(name?: string): AutomationBuilder {
  return new AutomationBuilder(name);
}

/** Deterministically serialize a builder/plan into MCP's restricted dialect. */
export function canonicalAutomationCode(
  input: AutomationBuilder | AutomationPlan,
): string {
  const plan = input instanceof AutomationBuilder ? input.toPlan() : clonePlan(input);
  validatePlan(plan);
  return renderCanonicalCode(plan);
}

function renderCanonicalCode(plan: AutomationPlan): string {
  let code =
    plan.name === undefined
      ? "automation()"
      : `automation(${JSON.stringify(plan.name)})`;
  for (const step of plan.steps) {
    code += "\n  .";
    switch (step.type) {
      case "pause":
        code += step.tts === undefined ? "pause()" : `pause(${step.tts})`;
        break;
      case "key":
        code += `${step.down ? "keyDown" : "keyUp"}(${JSON.stringify(step.key)})`;
        break;
      case "press_key":
        code += `pressKey(${JSON.stringify(step.key)})`;
        break;
      case "mouse_move":
        code += `mouseMove(${step.x}, ${step.y})`;
        break;
      case "mouse_button":
        code += `${step.down ? "mouseDown" : "mouseUp"}(${JSON.stringify(step.button)})`;
        break;
      case "mouse_wheel":
        code += `mouseWheel(${step.delta})`;
        break;
      case "ui_click":
        code += `uiClick(${step.slot})`;
        break;
      case "step":
        code += `step({ frames: ${step.frames}, dts: ${step.dts} })`;
        break;
      case "inspect":
        code +=
          step.label === undefined
            ? "inspect()"
            : `inspect(${JSON.stringify(step.label)})`;
        break;
      case "expect_model":
        code += `expectModel(${JSON.stringify(step.path)}, ${JSON.stringify(step.equals)})`;
        break;
      case "expect_model_close":
        code += `expectModelClose(${JSON.stringify(step.path)}, ${step.expected}, ${step.abs_tolerance})`;
        break;
      case "capture":
        code +=
          step.label === undefined
            ? "capture()"
            : `capture(${JSON.stringify(step.label)})`;
        break;
    }
  }
  return `${code};\n`;
}

/**
 * Execute a builder or serialized plan through the existing typed SDK.
 *
 * This is the standalone twin of MCP's `run_automation_code`. It executes a
 * plan, not source; only the MCP server needs the restricted source parser.
 */
export async function runAutomation(
  client: AutomationClient,
  input: AutomationBuilder | AutomationPlan,
): Promise<AutomationRunResult> {
  const plan = input instanceof AutomationBuilder ? input.toPlan() : clonePlan(input);
  validatePlan(plan);
  const assertions: AutomationAssertion[] = [];
  const observations: AutomationObservation[] = [];
  const captures: AutomationCapture[] = [];

  for (const [index, step] of plan.steps.entries()) {
    try {
      switch (step.type) {
        case "pause":
          await client.pause(step.tts);
          break;
        case "key":
          await (step.down ? client.keyDown(step.key) : client.keyUp(step.key));
          break;
        case "press_key": {
          let actionError: unknown;
          try {
            await client.keyDown(step.key);
          } catch (error) {
            actionError = error;
          }
          if (actionError === undefined) {
            try {
              await client.stepFrames(1, AUTOMATION_DEFAULT_STEP_DT);
            } catch (error) {
              actionError = error;
            }
          }
          // A timeout/error response may still have applied keyDown. Release
          // is therefore best-effort even when keyDown itself reports failure.
          try {
            await client.keyUp(step.key);
          } catch (releaseError) {
            if (actionError !== undefined) {
              throw new Error(
                `${String(actionError)}; best-effort key release also failed: ${String(releaseError)}`,
              );
            }
            throw releaseError;
          }
          if (actionError !== undefined) throw actionError;
          break;
        }
        case "mouse_move":
          await client.mouseMove(step.x, step.y);
          break;
        case "mouse_button":
          await (step.down
            ? client.mouseDown(step.button)
            : client.mouseUp(step.button));
          break;
        case "mouse_wheel":
          await client.mouseWheel(step.delta);
          break;
        case "ui_click":
          await client.input({
            type: "ui_event",
            slot: step.slot,
            kind: "Clicked",
          });
          break;
        case "step":
          await client.stepFrames(step.frames, step.dts);
          break;
        case "inspect":
          observations.push({ label: step.label, state: await client.state() });
          break;
        case "expect_model": {
          const state = await client.state();
          const actual = modelValueAt(state.model, step.path);
          if (actual === undefined) {
            throw new Error(
              `model assertion path ${JSON.stringify(step.path)} does not exist`,
            );
          }
          if (!jsonEqual(actual, step.equals)) {
            throw new Error(
              `model assertion failed at ${JSON.stringify(step.path)}: expected ${JSON.stringify(step.equals)}, got ${JSON.stringify(actual)}`,
            );
          }
          assertions.push({
            path: step.path,
            expected: step.equals,
            actual,
            passed: true,
          });
          break;
        }
        case "expect_model_close": {
          const state = await client.state();
          const actual = modelValueAt(state.model, step.path);
          if (actual === undefined) {
            throw new Error(
              `model assertion path ${JSON.stringify(step.path)} does not exist`,
            );
          }
          if (typeof actual !== "number" || !Number.isFinite(actual)) {
            throw new Error(
              `numeric model assertion at ${JSON.stringify(step.path)} requires a numeric value, got ${JSON.stringify(actual)}`,
            );
          }
          if (Math.abs(actual - step.expected) > step.abs_tolerance) {
            throw new Error(
              `numeric model assertion failed at ${JSON.stringify(step.path)}: expected ${step.expected} ± ${step.abs_tolerance}, got ${actual}`,
            );
          }
          assertions.push({
            path: step.path,
            expected: step.expected,
            actual,
            absTolerance: step.abs_tolerance,
            passed: true,
          });
          break;
        }
        case "capture":
          captures.push({ label: step.label, png: await client.capture() });
          break;
      }
    } catch (error) {
      throw new Error(
        `automation step ${index + 1} (${step.type}) failed: ${String(error)}`,
        { cause: error },
      );
    }
  }

  return {
    plan,
    assertions,
    observations,
    captures,
    finalState: await client.state(),
  };
}

function clonePlan(plan: AutomationPlan): AutomationPlan {
  return structuredClone(plan);
}

function validatePlan(plan: AutomationPlan): void {
  if (plan.version !== AUTOMATION_PLAN_VERSION) {
    throw new Error(
      `automation plan version must be ${AUTOMATION_PLAN_VERSION}, got ${String(plan.version)}`,
    );
  }
  if (
    plan.name !== undefined &&
    (plan.name.length === 0 || utf8Length(plan.name) > 80)
  ) {
    throw new Error("automation name must contain 1–80 UTF-8 bytes");
  }
  if (plan.steps.length === 0 || plan.steps.length > AUTOMATION_MAX_STEPS) {
    throw new Error(
      `automation plan must contain 1–${AUTOMATION_MAX_STEPS} steps`,
    );
  }
  const captures = plan.steps.filter((step) => step.type === "capture").length;
  if (captures > AUTOMATION_MAX_CAPTURES) {
    throw new Error(
      `automation plan has ${captures} captures; maximum is ${AUTOMATION_MAX_CAPTURES}`,
    );
  }
  const totalFrames = plan.steps.reduce(
    (total, step) =>
      total +
      (step.type === "step"
        ? step.frames
        : step.type === "press_key"
          ? 1
          : 0),
    0,
  );
  if (
    !Number.isSafeInteger(totalFrames) ||
    totalFrames > AUTOMATION_MAX_TOTAL_FRAMES
  ) {
    throw new Error(
      `automation plan requests ${totalFrames} frames; maximum is ${AUTOMATION_MAX_TOTAL_FRAMES}`,
    );
  }
  for (const step of plan.steps) validateStep(step);
  const sourceBytes = utf8Length(renderCanonicalCode(plan));
  if (sourceBytes > AUTOMATION_MAX_SOURCE_BYTES) {
    throw new Error(
      `canonical automation source is ${sourceBytes} bytes; maximum is ${AUTOMATION_MAX_SOURCE_BYTES}`,
    );
  }
}

function validateStep(step: AutomationStep): void {
  const finite = (name: string, value: number) => {
    if (!Number.isFinite(value)) throw new Error(`${name} must be finite`);
  };
  switch (step.type) {
    case "pause":
      if (step.tts !== undefined) {
        finite("pause tts", step.tts);
        if (step.tts < 0) throw new Error("pause tts must be non-negative");
      }
      break;
    case "key":
    case "press_key":
      if (!step.key || utf8Length(step.key) > 32) {
        throw new Error("key must contain 1–32 UTF-8 bytes");
      }
      if (!validKeyName(step.key)) {
        throw new Error(
          `unknown key ${JSON.stringify(step.key)}; expected A-Z, Up, Down, Left, Right, Space, Enter, Escape, or 0-9`,
        );
      }
      break;
    case "mouse_move":
      signedI32("mouse x", step.x);
      signedI32("mouse y", step.y);
      break;
    case "mouse_button":
      if (!["left", "right", "middle"].includes(step.button)) {
        throw new Error(`unknown mouse button ${String(step.button)}`);
      }
      break;
    case "mouse_wheel":
      signedI32("mouse wheel delta", step.delta);
      break;
    case "ui_click":
      if (
        !Number.isSafeInteger(step.slot) ||
        step.slot < 0 ||
        step.slot > 0xffff_ffff
      ) {
        throw new Error("UI slot must be an unsigned 32-bit integer");
      }
      break;
    case "step":
      if (
        !Number.isSafeInteger(step.frames) ||
        step.frames < 1 ||
        step.frames > AUTOMATION_MAX_TOTAL_FRAMES
      ) {
        throw new Error("step frames are outside the plan budget");
      }
      finite("step dts", step.dts);
      if (step.dts <= 0 || step.dts > 1) {
        throw new Error("step dts must be greater than 0 and at most 1");
      }
      break;
    case "inspect":
    case "capture":
      if (
        step.label !== undefined &&
        (!step.label || utf8Length(step.label) > 80)
      ) {
        throw new Error("label must contain 1–80 UTF-8 bytes");
      }
      break;
    case "expect_model":
      validateModelPath(step.path);
      validateLiteral(step.equals);
      if (literalDepth(step.equals) > AUTOMATION_MAX_LITERAL_DEPTH) {
        throw new Error(
          `expected model value exceeds depth ${AUTOMATION_MAX_LITERAL_DEPTH}`,
        );
      }
      break;
    case "expect_model_close":
      validateModelPath(step.path);
      finite("expected model value", step.expected);
      finite("absolute tolerance", step.abs_tolerance);
      if (step.abs_tolerance < 0) {
        throw new Error("absolute tolerance must be non-negative");
      }
      break;
    default:
      throw new Error(
        `unknown automation step ${String((step as { type?: unknown }).type)}`,
      );
  }
}

function validKeyName(key: string): boolean {
  const normalized = key.toLowerCase();
  return (
    /^[a-z0-9]$/.test(normalized) ||
    ["up", "down", "left", "right", "space", "enter", "escape"].includes(
      normalized,
    )
  );
}

function signedI32(name: string, value: number): void {
  if (
    !Number.isSafeInteger(value) ||
    value < -0x8000_0000 ||
    value > 0x7fff_ffff
  ) {
    throw new Error(`${name} must be a signed 32-bit integer`);
  }
}

function validateModelPath(path: string): void {
  if (utf8Length(path) > 256) throw new Error("model path is too long");
  if (
    !path.startsWith("/") &&
    path !== "" &&
    (path.split(".").some((segment) => segment === "") ||
      path
        .split(".")
        .some((segment) =>
          ["__proto__", "prototype", "constructor"].includes(segment),
        ))
  ) {
    throw new Error(
      "dotted model paths cannot contain empty or prototype-related segments",
    );
  }
}

function utf8Length(text: string): number {
  return Buffer.byteLength(text, "utf8");
}

function validateLiteral(value: ModelJson): void {
  if (typeof value === "number" && !Number.isFinite(value)) {
    throw new Error("model assertion literals must contain only finite numbers");
  }
  if (Array.isArray(value)) {
    value.forEach(validateLiteral);
  } else if (value !== null && typeof value === "object") {
    Object.values(value).forEach(validateLiteral);
  }
}

function literalDepth(value: ModelJson): number {
  if (value === null || typeof value !== "object") return 0;
  if (Array.isArray(value)) {
    return 1 + Math.max(0, ...value.map(literalDepth));
  }
  return 1 + Math.max(0, ...Object.values(value).map(literalDepth));
}

function modelValueAt(model: ModelJson, path: string): ModelJson | undefined {
  if (path === "") return model;
  const segments = path.startsWith("/")
    ? path
        .slice(1)
        .split("/")
        .map((segment) => segment.replace(/~1/g, "/").replace(/~0/g, "~"))
    : path.split(".");
  let value: ModelJson | undefined = model;
  for (const segment of segments) {
    if (Array.isArray(value)) {
      if (!/^(0|[1-9]\d*)$/.test(segment)) return undefined;
      value = value[Number(segment)];
    } else if (value !== null && typeof value === "object") {
      if (!Object.prototype.hasOwnProperty.call(value, segment)) return undefined;
      value = value[segment];
    } else {
      return undefined;
    }
  }
  return value;
}

function jsonEqual(left: ModelJson, right: ModelJson): boolean {
  if (Object.is(left, right)) return true;
  if (Array.isArray(left) && Array.isArray(right)) {
    return (
      left.length === right.length &&
      left.every((value, index) => jsonEqual(value, right[index]))
    );
  }
  if (
    left !== null &&
    right !== null &&
    typeof left === "object" &&
    typeof right === "object" &&
    !Array.isArray(left) &&
    !Array.isArray(right)
  ) {
    const leftKeys = Object.keys(left);
    const rightKeys = Object.keys(right);
    return (
      leftKeys.length === rightKeys.length &&
      leftKeys.every(
        (key) =>
          Object.prototype.hasOwnProperty.call(right, key) &&
          jsonEqual(left[key], right[key]),
      )
    );
  }
  return false;
}
