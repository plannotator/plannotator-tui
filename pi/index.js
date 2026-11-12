import path from "node:path";
import { startSessionServer } from "./session-server.js";

function readSnapshot(ctx, pid) {
  const manager = ctx.sessionManager;
  const sessionId = manager.getSessionId();
  const sessionFile = manager.getSessionFile();
  const leafId = manager.getLeafId();

  if (typeof sessionId !== "string") {
    throw new TypeError("Pi returned an invalid session ID");
  }
  if (sessionFile != null && (typeof sessionFile !== "string" || !path.isAbsolute(sessionFile))) {
    throw new TypeError("Pi returned a non-absolute session path");
  }
  if (leafId != null && typeof leafId !== "string") {
    throw new TypeError("Pi returned an invalid session leaf ID");
  }

  return {
    version: 1,
    pid,
    session_id: sessionId,
    session_path: sessionFile ?? null,
    leaf_id: leafId ?? null,
  };
}

/** Register the endpoint. Environment values are injectable for isolated lifecycle tests. */
export function registerPiSessionEndpoint(
  pi,
  { home = () => process.env.HOME, pid = () => process.pid, platform = () => process.platform } = {},
) {
  let endpoint;

  pi.on("session_start", async (_event, ctx) => {
    if (ctx.mode !== "tui" || platform() === "win32") return;
    if (endpoint) throw new Error("Pi session socket is already running");

    const instancePid = pid();
    endpoint = await startSessionServer({
      home: home(),
      pid: instancePid,
      getSession: () => readSnapshot(ctx, instancePid),
    });
  });

  pi.on("session_shutdown", async () => {
    const current = endpoint;
    endpoint = undefined;
    await current?.close();
  });
}

export default function (pi) {
  registerPiSessionEndpoint(pi);
}
