// Pre-download the VS Code build the fixture launches, so the first test does
// not eat the ~150MB download inside its own timeout. The binary is cached
// under .vscode-test/, so the fixture's own downloadAndUnzipVSCode call returns
// the cached path instantly.
import { downloadAndUnzipVSCode } from "@vscode/test-electron/out/download.js";

export default async function globalSetup() {
  const version = process.env.FUNCTOR_VSCODE_VERSION || "stable";
  await downloadAndUnzipVSCode(version);
}
