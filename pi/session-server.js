import { constants } from "node:fs";
import { chmod, lstat, mkdir, unlink } from "node:fs/promises";
import net from "node:net";
import path from "node:path";

const SOCKET_MODE = 0o600;
const PRIVATE_DIRECTORY_MODE = 0o700;
const CONNECT_PROBE_TIMEOUT_MS = 250;

function endpointError(code, message) {
  const error = new Error(message);
  error.code = code;
  return error;
}

function sameFile(left, right) {
  return left.dev === right.dev && left.ino === right.ino;
}

function isSocket(stat) {
  return (stat.mode & constants.S_IFMT) === constants.S_IFSOCK;
}

async function lstatIfPresent(filePath) {
  try {
    return await lstat(filePath);
  } catch (error) {
    if (error.code === "ENOENT") return undefined;
    throw error;
  }
}

async function ensureDirectory(directory, uid, { privateDirectory = false } = {}) {
  try {
    await mkdir(directory, { mode: PRIVATE_DIRECTORY_MODE });
  } catch (error) {
    if (error.code !== "EEXIST") throw error;
  }

  const stat = await lstat(directory);
  if (stat.isSymbolicLink()) {
    throw endpointError("ELOOP", `Refusing symlink in Pi socket directory: ${directory}`);
  }
  if (!stat.isDirectory()) {
    throw endpointError("ENOTDIR", `Pi socket path component is not a directory: ${directory}`);
  }
  if (stat.uid !== uid) {
    throw endpointError("EPERM", `Pi socket directory is not owned by the current user: ${directory}`);
  }

  const unsafeBits = privateDirectory ? 0o077 : 0o022;
  if ((stat.mode & unsafeBits) !== 0) {
    const requirement = privateDirectory ? "private (mode 0700)" : "not group- or world-writable";
    throw endpointError("EPERM", `Pi socket directory must be ${requirement}: ${directory}`);
  }
}

async function ensurePrivateSocketDirectory(home, uid) {
  if (typeof home !== "string" || !path.isAbsolute(home)) {
    throw endpointError("EINVAL", "HOME must be an absolute path for the Pi session socket");
  }

  const absoluteHome = path.resolve(home);
  const homeStat = await lstat(absoluteHome);
  if (homeStat.isSymbolicLink()) {
    throw endpointError("ELOOP", "HOME must not be a symlink for the Pi session socket");
  }
  if (!homeStat.isDirectory()) {
    throw endpointError("ENOTDIR", "HOME is not a directory for the Pi session socket");
  }
  if (homeStat.uid !== uid || (homeStat.mode & 0o022) !== 0) {
    throw endpointError("EPERM", "HOME must be owned by the current user and not group- or world-writable");
  }

  const cacheDirectory = path.join(absoluteHome, ".cache");
  const appDirectory = path.join(cacheDirectory, "plannotator-tui");
  const socketDirectory = path.join(appDirectory, "pi");
  await ensureDirectory(cacheDirectory, uid);
  await ensureDirectory(appDirectory, uid);
  await ensureDirectory(socketDirectory, uid, { privateDirectory: true });
  return socketDirectory;
}

function probeSocket(socketPath) {
  return new Promise((resolve) => {
    const socket = net.createConnection({ path: socketPath });
    let settled = false;
    const timer = setTimeout(() => finish("unknown"), CONNECT_PROBE_TIMEOUT_MS);
    timer.unref?.();

    function finish(result) {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      socket.destroy();
      resolve(result);
    }

    socket.once("connect", () => finish("active"));
    socket.once("error", (error) => {
      if (error.code === "ECONNREFUSED" || error.code === "ENOENT") {
        finish("stale");
      } else {
        finish("unknown");
      }
    });
  });
}

async function removeStaleSocket(socketPath, uid) {
  const existing = await lstatIfPresent(socketPath);
  if (!existing) return;
  if (existing.isSymbolicLink()) {
    throw endpointError("ELOOP", `Refusing to replace a symlink at ${socketPath}`);
  }
  if (!isSocket(existing)) {
    throw endpointError("EEXIST", `Refusing to replace a non-socket at ${socketPath}`);
  }
  if (existing.uid !== uid) {
    throw endpointError("EPERM", `Refusing to replace a socket owned by another user at ${socketPath}`);
  }

  const state = await probeSocket(socketPath);
  if (state === "active") {
    throw endpointError("EADDRINUSE", `A Pi session socket is already active at ${socketPath}`);
  }
  if (state !== "stale") {
    throw endpointError("EBUSY", `Could not safely determine whether the Pi socket is stale: ${socketPath}`);
  }

  const current = await lstatIfPresent(socketPath);
  if (!current) return;
  if (current.isSymbolicLink() || !isSocket(current) || current.uid !== uid || !sameFile(existing, current)) {
    throw endpointError("EBUSY", `Pi socket changed while checking whether it was stale: ${socketPath}`);
  }
  await unlink(socketPath);
}

function listen(server, socketPath) {
  return new Promise((resolve, reject) => {
    const onError = (error) => {
      server.off("listening", onListening);
      reject(error);
    };
    const onListening = () => {
      server.off("error", onError);
      resolve();
    };

    server.once("error", onError);
    server.once("listening", onListening);
    server.listen(socketPath);
  });
}

async function closeServer(server) {
  await new Promise((resolve, reject) => {
    server.close((error) => (error ? reject(error) : resolve()));
  });
}

async function unlinkOwnedSocket(socketPath, ownedStat, uid) {
  const current = await lstatIfPresent(socketPath);
  if (!current) return;
  if (current.isSymbolicLink() || !isSocket(current) || current.uid !== uid || !sameFile(ownedStat, current)) {
    throw endpointError("EBUSY", `Pi socket path changed; refusing to remove it: ${socketPath}`);
  }
  await unlink(socketPath);
}

/**
 * Start the read-only Unix socket used by the Pi extension.
 * The callback is called for every accepted connection so session data is live.
 */
export async function startSessionServer({ home, pid, getSession, onError = defaultErrorHandler }) {
  if (process.platform === "win32" || typeof process.getuid !== "function") {
    throw endpointError("ENOTSUP", "The Pi session socket is supported on Unix only");
  }
  if (!Number.isSafeInteger(pid) || pid <= 0) {
    throw endpointError("EINVAL", "The Pi session socket PID must be a positive integer");
  }
  if (typeof getSession !== "function") {
    throw endpointError("EINVAL", "A live Pi session callback is required");
  }

  const uid = process.getuid();
  const directory = await ensurePrivateSocketDirectory(home, uid);
  const socketPath = path.join(directory, `${pid}.sock`);
  await removeStaleSocket(socketPath, uid);

  const sockets = new Set();
  const server = net.createServer((socket) => {
    sockets.add(socket);
    socket.once("close", () => sockets.delete(socket));
    socket.on("error", () => {});

    let response;
    try {
      response = `${JSON.stringify(getSession())}\n`;
    } catch {
      onError(endpointError("ESESSION", "Could not read the current Pi session"));
      socket.destroy();
      return;
    }
    socket.end(response, () => socket.destroy());
  });

  let ownedStat;
  try {
    await listen(server, socketPath);
    const boundStat = await lstat(socketPath);
    if (!isSocket(boundStat) || boundStat.uid !== uid) {
      throw endpointError("EPERM", `Could not validate the Pi socket created at ${socketPath}`);
    }
    ownedStat = boundStat;
    await chmod(socketPath, SOCKET_MODE);
    const chmodStat = await lstat(socketPath);
    if (!sameFile(ownedStat, chmodStat) || !isSocket(chmodStat) || chmodStat.uid !== uid) {
      throw endpointError("EBUSY", `Pi socket path changed during setup: ${socketPath}`);
    }
    server.on("error", (error) => onError(error));
  } catch (error) {
    for (const socket of sockets) socket.destroy();
    if (server.listening) await closeServer(server);
    if (ownedStat) await unlinkOwnedSocket(socketPath, ownedStat, uid);
    throw error;
  }

  let closePromise;
  return {
    path: socketPath,
    close() {
      if (!closePromise) {
        closePromise = (async () => {
          for (const socket of sockets) socket.destroy();
          await closeServer(server);
          await unlinkOwnedSocket(socketPath, ownedStat, uid);
        })();
      }
      return closePromise;
    },
  };
}

function defaultErrorHandler(error) {
  const code = typeof error.code === "string" ? error.code : "unknown";
  console.error(`[plannotator-tui] Pi session socket error (${code})`);
}
