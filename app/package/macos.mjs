// Assembles target/package/Functor.app: the desktop app as an ad-hoc-signed
// macOS bundle for the machine that built it (no Developer ID, no
// notarization).
//
//   node app/package/macos.mjs                  builds the release `functor` first
//   node app/package/macos.mjs --functor F [--out DIR]   bundles this one
//
// Layout: Contents/MacOS/functor, the one executable (D40): built with the
// app in (`npm run build:cli` on macOS), so opened with no arguments it is
// the app, and its game sessions and tool servers are itself again. They run
// it by Contents/Helpers/functor, a symbolic link to it: from outside MacOS
// they do not take the app's bundle id, so a quit sent to that id reaches the
// app (a hidden game also declares itself a background process: no Dock
// tile). Contents/Resources/Functor.icns.
import { execFileSync } from "node:child_process";
import { cpSync, mkdirSync, mkdtempSync, readFileSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { parseArgs } from "node:util";
import { Resvg } from "@resvg/resvg-js";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const run = (cmd, args, opts) => execFileSync(cmd, args, { stdio: "inherit", ...opts });

const { values: opts } = parseArgs({ options: { functor: { type: "string" }, out: { type: "string" } } });
if (!opts.functor) run("npm", ["run", "build:cli"], { cwd: root });
const functor = resolve(opts.functor ?? join(root, "target/release/functor"));
const bundle = join(resolve(opts.out ?? join(root, "target/package")), "Functor.app");

rmSync(bundle, { recursive: true, force: true });
for (const dir of ["MacOS", "Helpers", "Resources"]) mkdirSync(join(bundle, "Contents", dir), { recursive: true });
cpSync(functor, join(bundle, "Contents/MacOS/functor"));
symlinkSync("../MacOS/functor", join(bundle, "Contents/Helpers/functor"));

// One wording for the mic prompt: the plist the bare binary embeds.
const embedded = join(root, "app/functor-app/Info.plist");
const read = (key) =>
  execFileSync("plutil", ["-extract", key, "raw", "-o", "-", embedded], { encoding: "utf8" }).trim();
// Bundle versions are up to three integers; `0.0.0-dev` → `0.0.0`.
const cliVersion = readFileSync(join(root, "cli/Cargo.toml"), "utf8").match(/^version = "(.+)"/m)[1];
const version = (process.env.FUNCTOR_RELEASE_VERSION || cliVersion).replace(/^v/, "").match(/^\d+(\.\d+){0,2}/)[0];
const info = join(bundle, "Contents/Info.plist");
writeFileSync(
  info,
  JSON.stringify({
    CFBundleDevelopmentRegion: "en",
    CFBundleDisplayName: "Functor",
    CFBundleExecutable: "functor",
    CFBundleIconFile: "Functor",
    CFBundleIdentifier: read("CFBundleIdentifier"),
    CFBundleInfoDictionaryVersion: "6.0",
    CFBundleName: read("CFBundleName"),
    CFBundlePackageType: "APPL",
    CFBundleShortVersionString: version,
    CFBundleVersion: version,
    LSApplicationCategoryType: "public.app-category.developer-tools",
    LSMinimumSystemVersion: "11.0",
    NSHighResolutionCapable: true,
    NSMicrophoneUsageDescription: read("NSMicrophoneUsageDescription"),
    // The device link listens on the LAN and advertises over mDNS; a bundled
    // app is asked for local-network access with this.
    NSLocalNetworkUsageDescription:
      "Functor finds your headset on this network and sends it the game you are working on.",
    NSBonjourServices: ["_functor-link._tcp"],
  }),
);
run("plutil", ["-convert", "xml1", info]);

// The icon: every iconset size from the committed SVG.
const svg = readFileSync(join(root, "app/package/icon.svg"), "utf8");
const iconset = join(mkdtempSync(join(tmpdir(), "functor-icon-")), "Functor.iconset");
mkdirSync(iconset);
for (const size of [16, 32, 128, 256, 512]) {
  for (const scale of [1, 2]) {
    const png = new Resvg(svg, { fitTo: { mode: "width", value: size * scale } }).render().asPng();
    writeFileSync(join(iconset, `icon_${size}x${size}${scale === 2 ? "@2x" : ""}.png`), png);
  }
}
run("iconutil", ["-c", "icns", iconset, "-o", join(bundle, "Contents/Resources/Functor.icns")]);
rmSync(dirname(iconset), { recursive: true, force: true });

// Ad hoc: Apple silicon runs only signed code, and the mic and local-network
// permissions attach to the signature.
run("codesign", ["--force", "--deep", "--sign", "-", bundle]);
run("codesign", ["--verify", "--deep", "--strict", bundle]);
console.log(`\n${bundle}`);
