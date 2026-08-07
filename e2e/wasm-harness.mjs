// Shared boot for the exported-bundle wasm e2es (module-role, gamepad,
// touch): `build wasm` the fixture with the built CLI, serve `dist/web` from
// an ephemeral port (hermetic next to anyone's dev server on :8080), and
// launch software-WebGL2 chromium (no real GPU needed; these checks compare
// no pixels).
import { execFileSync } from "node:child_process";
import { createReadStream, statSync } from "node:fs";
import http from "node:http";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "@playwright/test";

export const ROOT = fileURLToPath(new URL("..", import.meta.url));
export const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const TYPES = {
  ".html": "text/html",
  ".js": "text/javascript",
  ".wasm": "application/wasm",
  ".fun": "text/plain",
};

/** Export `dir` with `build wasm` and serve its dist/web on an ephemeral
 * port. Returns `{ server, port }`; `server.close()` when done. */
export async function serveExportedBundle(dir) {
  console.log(
    execFileSync(path.join(ROOT, "target/debug/functor"), ["-d", dir, "build", "wasm"], {
      encoding: "utf8",
    }),
  );
  const root = path.join(dir, "dist", "web");
  const server = http.createServer((req, res) => {
    let rel = decodeURIComponent(req.url.split("?")[0]);
    if (rel === "/") rel = "/index.html";
    const file = path.join(root, rel);
    try {
      statSync(file);
    } catch {
      res.writeHead(404).end("not found");
      return;
    }
    res.writeHead(200, {
      "Content-Type": TYPES[path.extname(file)] ?? "application/octet-stream",
    });
    createReadStream(file).pipe(res);
  });
  await new Promise((r) => server.listen(0, "127.0.0.1", r));
  return { server, port: server.address().port };
}

/** Software-WebGL2 chromium (swiftshader) that comes up on GPU-less CI. */
export function launchSoftwareGL() {
  return chromium.launch({
    args: [
      "--use-gl=angle",
      "--use-angle=swiftshader",
      "--enable-unsafe-swiftshader",
      "--ignore-gpu-blocklist",
    ],
  });
}

/** Poll `log` until a line matches `pattern`, else throw with the console. */
export async function waitFor(log, pattern, what, timeoutMs = 30000) {
  const until = Date.now() + timeoutMs;
  while (Date.now() < until) {
    if (log.some((line) => pattern.test(line))) return;
    await sleep(200);
  }
  throw new Error(`timed out waiting for ${what}\n--- console ---\n${log.join("\n")}`);
}

/** Print a ✓/✗ line; a failure sets the process exit code. */
export function expect(cond, what) {
  console.log(`  ${cond ? "✓" : "✗"} ${what}`);
  if (!cond) process.exitCode = 1;
}
