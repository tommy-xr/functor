#!/usr/bin/env node

/** Try a PR's CI build without building it locally:
 *
 *   npm run pr -- <n> [--apk] [--cli] [--run <example>] [--allow-uninstall]
 *
 * Finds the successful `Build Android` / `Build CLI` run for the PR's head
 * commit (refusing a missing, running or failed one), downloads its artifacts
 * once into ~/.functor/pr-artifacts/<sha>/, then:
 *
 * - `--apk` installs the Quest APK (`adb install -r -g`) and launches it.
 *   An install signed with another key or versioned above the CI build must
 *   be uninstalled first, which `--allow-uninstall` permits.
 * - `--cli` prints the path of this host's `functor` binary.
 * - `--run <example>` (implies `--cli`) runs that binary on this checkout's
 *   `examples/<example>`: `run vr` with `--apk`, else `run native`.
 *
 * With neither `--apk` nor `--cli`, both. Needs `gh`, and `adb` for `--apk`.
 */

import { spawnSync } from "node:child_process";
import { existsSync } from "node:fs";
import { join } from "node:path";
import { apkPath, cliPath, cliPlatform, installAndLaunch, prHead, ROOT } from "./pr-builds.mjs";

function parseArgs(argv) {
  const options = { apk: false, cli: false, allowUninstall: false };
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    if (arg === "--apk") options.apk = true;
    else if (arg === "--cli") options.cli = true;
    else if (arg === "--allow-uninstall") options.allowUninstall = true;
    else if (arg === "--run") {
      options.run = argv[++i];
      if (!options.run) throw new Error("--run requires an example name");
    } else if (/^[1-9]\d*$/.test(arg) && options.pr === undefined) options.pr = arg;
    else throw new Error(`unknown argument: ${arg}`);
  }
  if (options.pr === undefined) {
    throw new Error("usage: npm run pr -- <n> [--apk] [--cli] [--run <example>] [--allow-uninstall]");
  }
  if (options.run) options.cli = true;
  // Neither asked for: both, skipping a CLI this host has no build of.
  if (!options.apk && !options.cli) options.apk = options.cli = options.defaulted = true;
  return options;
}

function main() {
  const options = parseArgs(process.argv.slice(2));
  const example = options.run && join(ROOT, "examples", options.run);
  if (example && !existsSync(join(example, "functor.json"))) {
    throw new Error(`no example at ${example}`);
  }
  const head = prHead(options.pr);
  console.log(`PR #${options.pr} ${head.branch} @ ${head.sha}`);

  // Every download precedes any device change, so a refusal touches nothing.
  const apk = options.apk && apkPath(head);
  const platform = cliPlatform();
  let cli;
  if (options.cli && !platform) {
    const message = `CI builds no functor CLI for ${process.platform}-${process.arch}`;
    if (!options.defaulted) throw new Error(message);
    console.log(`${message}; skipping it`);
  } else if (options.cli) {
    cli = cliPath(head, platform);
    console.log(`functor CLI: ${cli}`);
  }

  const serial = apk && installAndLaunch(apk, options.allowUninstall);
  if (example) {
    const result = spawnSync(cli, ["-d", example, "run", apk ? "vr" : "native"], {
      stdio: "inherit",
      env: serial ? { ...process.env, ANDROID_SERIAL: serial } : process.env,
    });
    if (result.error) throw result.error;
    process.exitCode = result.status ?? 1;
  }
}

try {
  main();
} catch (error) {
  console.error(`pr: ${error instanceof Error ? error.message : error}`);
  process.exitCode = 1;
}
