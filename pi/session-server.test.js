import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { chmod, lstat, mkdir, mkdtemp, readFile, realpath, rm, symlink, writeFile } from "node:fs/promises";
import net from "node:net";
import path from "node:path";
import { after, test } from "node:test";
import { registerPiSessionEndpoint } from "./index.js";
import { startSessionServer } from "./session-server.js";

const temporaryHomes = new Set();

after(async () => {
  await Promise.all([...temporaryHomes].map((home) => rm(home, { recursive: true, force: true })));
});

async function makeHome() {
  // macOS has a short Unix socket path limit and its default TMPDIR can be long.
  const home = await mkdtemp(path.join(await realpath("/tmp"), "plannotator-pi-test-"));
  temporaryHomes.add(home);
  return home;
}

function socketPath(home, pid) {
  return path.join(home, ".cache", "plannotator-tui", "pi", `${pid}.sock`);
}

async function getResponse(pathname) {
  return new Promise((resolve, reject) => {
    const socket = net.createConnection({ path: pathname });
    let response = "";
    socket.setEncoding("utf8");
    socket.on("data", (chunk) => (response += chunk));
    socket.once("end", () => resolve(response));
    socket.once("error", reject);
  });
}

function fakePi() {
  const handlers = new Map();
  return {
    on(event, handler) {
      handlers.set(event, handler);
    },
    emit(event, payload, ctx) {
      const handler = handlers.get(event);
      assert.ok(handler, `missing ${event} handler`);
      return handler(payload, ctx);
    },
  };
}

function sessionContext({ mode = "tui", sessionId, sessionFile, leafId }) {
  return {
    mode,
    sessionManager: {
      getSessionId: () => sessionId,
      getSessionFile: () => sessionFile,
      getLeafId: () => leafId,
    },
  };
}

test("each connection gets the current session snapshot as one newline-terminated JSON object", async (t) => {
  const home = await makeHome();
  const pid = process.pid + 41_000;
  let session = {
    version: 1,
    pid,
    session_id: "first-session",
    session_path: "/sessions/first.jsonl",
    leaf_id: "leaf-a",
  };
  const server = await startSessionServer({ home, pid, getSession: () => session });
  t.after(() => server.close());

  const first = await getResponse(server.path);
  assert.equal(first, `${JSON.stringify(session)}\n`);

  session = {
    version: 1,
    pid,
    session_id: "second-session",
    session_path: null,
    leaf_id: null,
  };
  const second = await getResponse(server.path);
  assert.equal(second, `${JSON.stringify(session)}\n`);
});

test("the extension is TUI-only and closes/reopens its socket across session lifecycle events", async () => {
  const home = await makeHome();
  const pid = process.pid + 42_000;
  const pi = fakePi();
  registerPiSessionEndpoint(pi, { home: () => home, pid: () => pid });

  await pi.emit("session_start", { reason: "startup" }, sessionContext({
    mode: "rpc",
    sessionId: "rpc-session",
    sessionFile: "/sessions/rpc.jsonl",
    leafId: "rpc-leaf",
  }));
  await assert.rejects(lstat(path.join(home, ".cache")), { code: "ENOENT" });

  const firstContext = sessionContext({
    sessionId: "session-before-reload",
    sessionFile: "/sessions/before.jsonl",
    leafId: "leaf-before",
  });
  await pi.emit("session_start", { reason: "startup" }, firstContext);
  const firstPath = socketPath(home, pid);
  assert.deepEqual(JSON.parse(await getResponse(firstPath)), {
    version: 1,
    pid,
    session_id: "session-before-reload",
    session_path: "/sessions/before.jsonl",
    leaf_id: "leaf-before",
  });

  firstContext.sessionManager.getLeafId = () => "leaf-after-tree-navigation";
  assert.equal(JSON.parse(await getResponse(firstPath)).leaf_id, "leaf-after-tree-navigation");

  await pi.emit("session_shutdown", { reason: "reload" }, firstContext);
  await assert.rejects(lstat(firstPath), { code: "ENOENT" });

  await pi.emit("session_start", { reason: "resume" }, sessionContext({
    sessionId: "resumed-session",
    sessionFile: undefined,
    leafId: null,
  }));
  assert.deepEqual(JSON.parse(await getResponse(firstPath)), {
    version: 1,
    pid,
    session_id: "resumed-session",
    session_path: null,
    leaf_id: null,
  });

  await pi.emit("session_shutdown", { reason: "quit" }, undefined);
  await assert.rejects(lstat(firstPath), { code: "ENOENT" });
});

test("the extension does not start a Unix socket on Windows", async () => {
  const home = await makeHome();
  const pi = fakePi();
  registerPiSessionEndpoint(pi, { home: () => home, platform: () => "win32" });
  await pi.emit("session_start", { reason: "startup" }, sessionContext({ sessionId: "session" }));
  await assert.rejects(lstat(path.join(home, ".cache")), { code: "ENOENT" });
});

test("an active socket belongs to its existing owner and is never replaced", async (t) => {
  const home = await makeHome();
  const pid = process.pid + 43_000;
  const first = await startSessionServer({
    home,
    pid,
    getSession: () => ({ version: 1, pid, session_id: "owner", session_path: null, leaf_id: null }),
  });
  t.after(() => first.close());

  await assert.rejects(
    startSessionServer({ home, pid, getSession: () => ({ session_id: "must-not-own" }) }),
    { code: "EADDRINUSE" },
  );
  assert.deepEqual(JSON.parse(await getResponse(first.path)), {
    version: 1,
    pid,
    session_id: "owner",
    session_path: null,
    leaf_id: null,
  });
});

test("a stale same-user socket is reclaimed after a refused connection", async (t) => {
  const home = await makeHome();
  const directory = path.join(home, ".cache", "plannotator-tui", "pi");
  await mkdir(directory, { recursive: true, mode: 0o700 });

  const script = [
    "const net = require('node:net');",
    "const path = require('node:path');",
    "const server = net.createServer();",
    "const socketPath = path.join(process.argv[1], process.pid + '.sock');",
    "server.listen(socketPath, () => process.kill(process.pid, 'SIGKILL'));",
  ].join("\n");
  const child = spawn(process.execPath, ["-e", script, directory], { stdio: "ignore" });
  const pid = child.pid;
  assert.ok(pid);
  const stalePath = socketPath(home, pid);
  const exit = await new Promise((resolve, reject) => {
    child.once("error", reject);
    child.once("exit", (code, signal) => resolve({ code, signal }));
  });
  assert.equal(exit.signal, "SIGKILL");
  assert.equal((await lstat(stalePath)).isSocket(), true);

  const server = await startSessionServer({
    home,
    pid,
    getSession: () => ({ version: 1, pid, session_id: "recovered", session_path: null, leaf_id: null }),
  });
  t.after(() => server.close());
  assert.equal(JSON.parse(await getResponse(server.path)).session_id, "recovered");
});

test("symlinks and non-socket files at the PID path are preserved", async () => {
  const home = await makeHome();
  const pid = process.pid + 45_000;
  const directory = path.dirname(socketPath(home, pid));
  await mkdir(directory, { recursive: true, mode: 0o700 });

  const target = path.join(home, "keep.txt");
  const link = socketPath(home, pid);
  await writeFile(target, "keep this file");
  await symlink(target, link);
  await assert.rejects(
    startSessionServer({ home, pid, getSession: () => ({}) }),
    { code: "ELOOP" },
  );
  assert.equal(await readFile(target, "utf8"), "keep this file");
  assert.equal((await lstat(link)).isSymbolicLink(), true);

  await rm(link);
  await writeFile(link, "not a socket");
  await assert.rejects(
    startSessionServer({ home, pid, getSession: () => ({}) }),
    { code: "EEXIST" },
  );
  assert.equal(await readFile(link, "utf8"), "not a socket");
});

test("a symlinked parent directory is rejected without writing through it", async () => {
  const home = await makeHome();
  const outside = await makeHome();
  const pid = process.pid + 47_000;
  const cache = path.join(home, ".cache");
  await mkdir(cache, { mode: 0o700 });
  await symlink(outside, path.join(cache, "plannotator-tui"));

  await assert.rejects(
    startSessionServer({ home, pid, getSession: () => ({}) }),
    { code: "ELOOP" },
  );
  await assert.rejects(lstat(path.join(outside, "pi")), { code: "ENOENT" });
});

test("the socket directory must be private and newly created socket permissions are restricted", async () => {
  const home = await makeHome();
  const pid = process.pid + 46_000;
  const expected = socketPath(home, pid);
  const server = await startSessionServer({
    home,
    pid,
    getSession: () => ({ version: 1, pid, session_id: "private", session_path: null, leaf_id: null }),
  });
  assert.equal(server.path, expected);
  assert.equal((await lstat(path.dirname(server.path))).mode & 0o777, 0o700);
  assert.equal((await lstat(server.path)).mode & 0o777, 0o600);
  await server.close();

  await chmod(path.dirname(expected), 0o755);
  await assert.rejects(
    startSessionServer({ home, pid, getSession: () => ({}) }),
    { code: "EPERM" },
  );
});
