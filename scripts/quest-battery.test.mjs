import assert from "node:assert/strict";
import test from "node:test";

import { batteryLine, parseDumpsysBattery } from "./quest-battery.mjs";

const reading = (overrides) => ({
  level_pct: 40,
  status: "discharging",
  plugged: true,
  current_now_ma: -1300,
  current_avg_ma: -1200,
  voltage_mv: 4000,
  ...overrides,
});

test("prints the averaged current and power from /state.battery", () => {
  assert.equal(
    batteryLine(reading(), reading({ level_pct: 39, current_avg_ma: -1250 })),
    "- Battery: 40% → 39% (discharging, plugged; avg current -1200 → -1250 mA ≈ -5.0 W)",
  );
});

test("falls back to the instantaneous current without an average", () => {
  assert.equal(
    batteryLine(
      reading({ current_avg_ma: null }),
      reading({ status: "charging", current_now_ma: 300, current_avg_ma: null, voltage_mv: null }),
    ),
    "- Battery: 40% → 40% (charging, plugged; now current -1300 → +300 mA)",
  );
});

test("an unreported level prints as ?", () => {
  assert.match(batteryLine(reading({ level_pct: null }), reading()), /^- Battery: \?% → 40% /);
});

test("dumpsys fallback parses state and reports no current", () => {
  const text = `Current Battery Service state:
  AC powered: false
  USB powered: true
  Wireless powered: false
  Dock powered: false
  Max charging current: 3000000
  Max charging voltage: 5000000
  Charge counter: 4648020
  status: 2
  health: 2
  present: true
  level: 92
  scale: 100
  voltage: 4093
  temperature: 287
`;
  const battery = parseDumpsysBattery(text);
  assert.deepEqual(battery, {
    level_pct: 92,
    status: "charging",
    plugged: true,
    current_now_ma: null,
    current_avg_ma: null,
    voltage_mv: 4093,
  });
  assert.equal(
    batteryLine(battery, battery),
    "- Battery: 92% → 92% (charging, plugged; net current n/a)",
  );
});
