// Resolve the command used to launch the functor-lang-lsp language server:
//
//   1. the `functor-lang.serverPath` setting, when set — explicit always wins
//   2. the binary bundled inside a platform-specific VSIX
//      (server/functor-lang-lsp[.exe], staged by the release pipeline)
//   3. bare "functor-lang-lsp", resolved from PATH (dev checkouts — see
//      ../README.md for how to get it there)
//
// Pure decision logic with fs injected, node-tested like inspector.js.
const path = require("path");

function resolveServerCommand(configured, extensionPath, platform, existsSync) {
  if (configured) return configured;
  const bundled = path.join(
    extensionPath,
    "server",
    platform === "win32" ? "functor-lang-lsp.exe" : "functor-lang-lsp"
  );
  if (existsSync(bundled)) return bundled;
  return "functor-lang-lsp";
}

module.exports = { resolveServerCommand };
