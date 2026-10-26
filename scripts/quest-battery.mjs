// Battery readings for `bench:quest`. The runtime's `/state.battery`
// (debug protocol v21) carries BatteryManager's current. `dumpsys battery`
// is the fallback for older runtimes: its charge counter is a cached,
// coarsely stepped snapshot, so no current is derived from it.

const STATUS = { 2: "charging", 3: "discharging", 4: "not charging", 5: "full" };

/** `/state.battery`'s shape from `dumpsys battery` text; currents are null. */
export function parseDumpsysBattery(text) {
  const number = (pattern) => {
    const match = text.match(pattern);
    return match ? Number(match[1]) : null;
  };
  return {
    level_pct: number(/level:\s+(\d+)/),
    status: STATUS[number(/status:\s+(\d+)/)] ?? "unknown",
    plugged: /(?:AC|USB|Wireless|Dock) powered:\s+true/.test(text),
    current_now_ma: null,
    current_avg_ma: null,
    // Anchored: `Max charging voltage:` precedes the battery `voltage:` line.
    voltage_mv: number(/^\s*voltage:\s+(\d+)/m),
  };
}

/** The report's `- Battery:` line from the sample's start and end readings. */
export function batteryLine(before, after) {
  // Prefer the device's averaged current; signed as the device reports it.
  const key =
    before.current_avg_ma != null && after.current_avg_ma != null
      ? "current_avg_ma"
      : "current_now_ma";
  const [from, to] = [before[key], after[key]];
  const signed = (ma) => `${ma >= 0 ? "+" : ""}${ma}`;
  let current = "net current n/a";
  if (from != null && to != null) {
    const label = key === "current_avg_ma" ? "avg" : "now";
    const watts =
      after.voltage_mv != null ? ` ≈ ${((to / 1000) * (after.voltage_mv / 1000)).toFixed(1)} W` : "";
    current = `${label} current ${signed(from)} → ${signed(to)} mA${watts}`;
  }
  return `- Battery: ${before.level_pct}% → ${after.level_pct}% (${after.status}, ${after.plugged ? "plugged" : "unplugged"}; ${current})`;
}
