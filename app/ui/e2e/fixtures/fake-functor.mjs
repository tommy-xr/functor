#!/usr/bin/env node
// A fake `functor` playing one game's debug server, for tests of what the
// app shows about a clock it did not pause itself. It answers the app's
// check (`app-compat`), then `run` serves `/state`'s clock and takes
// `POST /time` as the runtime does (`by` names who paused), so a test posts
// an agent's pause to the session's debug URL. No model, frames or input log.
import { createServer } from "node:http";

const args = process.argv.slice(2);
if (args.includes("app-compat")) {
  console.log(JSON.stringify({ app_protocol: 1000, version: "0.0.0-dev" }));
  process.exit(0);
}

let frame = 0;
let paused = args.includes("--paused");
let pausedBy = paused ? "start" : null;

const server = createServer((req, res) => {
  let body = "";
  req.on("data", (d) => (body += d));
  req.on("end", () => {
    if (req.method === "POST" && req.url === "/time") {
      const command = JSON.parse(body);
      paused = command.type !== "resume";
      pausedBy = paused ? (command.by ?? null) : null;
    }
    if (!paused) frame += 1;
    const clock = { paused, paused_by: pausedBy, recorded: { oldest: 0, newest: frame }, generation: 0, program_revision: 0 };
    const answer = req.url.startsWith("/input-log") ? { generation: 0, oldest: 0, newest: frame, steps: [] } : { frame, clock };
    // The app reads each answer to the connection's end.
    res.writeHead(200, { "content-type": "application/json", connection: "close" });
    res.end(JSON.stringify(answer));
  });
});
server.listen(0, "127.0.0.1", () => console.error(`[debug-server] listening on http://127.0.0.1:${server.address().port}`));
