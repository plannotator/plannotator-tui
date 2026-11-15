# Pi session endpoint

This package adds a read-only Unix socket for Rex's Plannotator launcher. It starts only in Pi's interactive TUI mode and listens at:

```text
$HOME/.cache/plannotator-tui/pi/<pid>.sock
```

The `pi` directory is created with mode `0700`; the socket is set to `0600`. The extension reads the current `ctx.sessionManager` values whenever a client connects, so `/tree` leaf changes are visible without restarting the endpoint. Pi's `session_shutdown` event closes the listener before session replacement, reload, or exit. The next `session_start` creates the endpoint for the active session.

## Protocol v1

The server sends one JSON object followed by `\n`, then closes the connection. It sends no greeting and expects no request data.

```json
{"version":1,"pid":12345,"session_id":"...","session_path":"/absolute/path.jsonl","leaf_id":"..."}
```

`session_path` is `null` until Pi persists a session file. `leaf_id` is `null` when there is no current entry. The `session_id` remains available for in-memory sessions.

Protocol limitations:

- There is no discovery service. A client must know the Pi process ID to form the socket path.
- Unix sockets are required; the extension does not start on Windows.
- Unix socket files do not record their creating process ID. On startup, the extension accepts an existing path only if it is a same-user socket and a connection proves it stale (`ECONNREFUSED` or `ENOENT`). Active, unowned, symlink, non-socket, or indeterminate paths are left untouched and startup fails. This prevents deleting an active endpoint, but after a crash the socket file alone cannot prove which historical process created it.
- The endpoint is local read-only IPC, not an authentication boundary between processes running as the same user.

## Install and test

Install the release package persistently with Pi:

```sh
pi install git:github.com/plannotator/plannotator-tui
```

Restart Pi, or run `/reload` in its TUI, to load the extension. It provides the
endpoint Rex needs for `plannotator-tui rex last` when the foreground app is Pi. The
endpoint runs only in Pi's interactive TUI on Unix; if it is unavailable, Rex reports
an install/enable error instead of guessing a session.

For local development, load the extension directly:

```sh
pi -e ./pi/index.js
```

Run the dependency-free tests with:

```sh
cd pi && npm test
```
