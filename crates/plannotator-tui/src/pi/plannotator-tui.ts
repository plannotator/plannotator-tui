/**
 * plannotator-tui link for Pi (docs/spec-pi-link.md in plannotator-tui).
 *
 * - Lists this Pi session in `<root>/pi/<session id>-<pid>.json` and listens on
 *   127.0.0.1 so plannotator-tui can send review feedback as a user message.
 * - Adds the tool `plannotator_tui_note`, which leaves a note in a running
 *   plannotator-tui that has the file open.
 * - Adds the command `/plannotator-tui`, which lists the running plannotator-tui apps.
 *
 * Install: `plannotator-tui pi install`, or copy this file to ~/.pi/agent/extensions/.
 * No dependencies beyond Node and Pi.
 */

import { randomBytes, timingSafeEqual } from "node:crypto";
import * as fs from "node:fs";
import * as net from "node:net";
import * as os from "node:os";
import * as path from "node:path";
import type { ExtensionAPI, ExtensionContext } from "@earendil-works/pi-coding-agent";
import { Type } from "typebox";

const VERSION = 1;
/** Longest request line we read (1 MiB, per the spec). */
const MAX_LINE = 1024 * 1024;
/** How long a client gets to send its request line. */
const READ_TIMEOUT_MS = 10_000;
/** plannotator-tui waits up to 5 s for its app to answer a note, after up to 5 s reading. */
const NOTE_TIMEOUT_MS = 12_000;
/** A send that started a turn counts as busy until `agent_start`, or this long. */
const STARTING_GRACE_MS = 10_000;

// ---------------------------------------------------------------------------
// Registry location and files

function expandHome(p: string): string {
	if (p === "~") return os.homedir();
	if (p.startsWith("~/") || p.startsWith("~\\")) return path.join(os.homedir(), p.slice(2));
	return p;
}

/** Plannotator's data dir: PLANNOTATOR_DATA_DIR → existing ~/.plannotator → absolute $XDG_DATA_HOME/plannotator → ~/.plannotator. */
export function plannotatorDataDir(env: NodeJS.ProcessEnv = process.env): string {
	const custom = env.PLANNOTATOR_DATA_DIR?.trim();
	if (custom) return expandHome(custom);
	const legacy = path.join(os.homedir(), ".plannotator");
	if (fs.existsSync(legacy)) return legacy;
	const xdg = env.XDG_DATA_HOME?.trim();
	if (xdg && path.isAbsolute(xdg)) return path.join(xdg, "plannotator");
	return legacy;
}

/** Registry root: PLANNOTATOR_TUI_SESSIONS_DIR, else `<data dir>/clients/plannotator-tui/sessions`. */
export function registryRoot(env: NodeJS.ProcessEnv = process.env): string {
	const custom = env.PLANNOTATOR_TUI_SESSIONS_DIR?.trim();
	if (custom) return path.resolve(custom);
	return path.join(plannotatorDataDir(env), "clients", "plannotator-tui", "sessions");
}

/** Write a file by renaming a 0600 temp file from the same folder over it. */
function writeAtomic(file: string, data: string): void {
	const dir = path.dirname(file);
	fs.mkdirSync(dir, { recursive: true, mode: 0o700 });
	const tmp = path.join(dir, `.${path.basename(file)}.${process.pid}.${randomBytes(4).toString("hex")}.tmp`);
	try {
		fs.writeFileSync(tmp, data, { mode: 0o600, flag: "wx" });
		fs.renameSync(tmp, file);
	} catch (err) {
		try {
			fs.unlinkSync(tmp);
		} catch {}
		throw err;
	}
}

function removeQuietly(file: string | undefined): void {
	if (!file) return;
	try {
		fs.unlinkSync(file);
	} catch {}
}

/** Signal 0 checks a pid without touching it. EPERM means it exists but is not ours. */
function pidAlive(pid: unknown): boolean {
	if (typeof pid !== "number" || !Number.isInteger(pid) || pid <= 0) return false;
	try {
		process.kill(pid, 0);
		return true;
	} catch (err) {
		return (err as NodeJS.ErrnoException).code === "EPERM";
	}
}

function realOr(p: string): string {
	try {
		return fs.realpathSync(p);
	} catch {
		return p;
	}
}

export interface TuiEntry {
	v: number;
	host: "plannotator-tui";
	pid: number;
	cwd: string;
	port: number;
	token: string;
	files: string[];
	updated: number;
}

/** Live plannotator-tui entries in `<root>/tui/`, newest first. Stale ones (dead pid) are deleted. */
export function liveTuiEntries(root: string = registryRoot()): TuiEntry[] {
	const dir = path.join(root, "tui");
	let names: string[];
	try {
		names = fs.readdirSync(dir);
	} catch {
		return [];
	}
	const out: TuiEntry[] = [];
	for (const name of names) {
		if (!name.endsWith(".json") || name.startsWith(".")) continue;
		const file = path.join(dir, name);
		let entry: TuiEntry;
		try {
			entry = JSON.parse(fs.readFileSync(file, "utf8"));
		} catch {
			continue; // half-written or foreign; leave it alone
		}
		if (!entry || entry.host !== "plannotator-tui" || typeof entry.port !== "number") continue;
		if (!pidAlive(entry.pid)) {
			removeQuietly(file);
			continue;
		}
		if (!Array.isArray(entry.files)) entry.files = [];
		out.push(entry);
	}
	return out.sort((a, b) => (b.updated ?? 0) - (a.updated ?? 0));
}

/** Entries that can take notes for `file` (absolute, real path): open file first, then any that list it. */
export function tuiTargetsFor(file: string, entries: TuiEntry[]): TuiEntry[] {
	const same = (p: unknown) => typeof p === "string" && (p === file || realOr(p) === file);
	const open = entries.filter((e) => same(e.files[0]));
	const listed = entries.filter((e) => !open.includes(e) && e.files.some(same));
	return [...open, ...listed];
}

// ---------------------------------------------------------------------------
// Wire: one JSON line each way over loopback TCP

class WireError extends Error {
	constructor(
		readonly code: "refused" | "timeout" | "bad_reply" | "io",
		message: string,
	) {
		super(message);
	}
}

/** Send one JSON line to 127.0.0.1:port and read one JSON line back. */
export function requestLine(port: number, message: object, timeoutMs = NOTE_TIMEOUT_MS): Promise<any> {
	return new Promise((resolve, reject) => {
		const conn = net.connect({ host: "127.0.0.1", port });
		let buf = "";
		let settled = false;
		const finish = (err: Error | null, value?: unknown) => {
			if (settled) return;
			settled = true;
			clearTimeout(timer);
			conn.destroy();
			if (err) reject(err);
			else resolve(value);
		};
		const timer = setTimeout(() => finish(new WireError("timeout", "no answer in time")), timeoutMs);
		conn.setEncoding("utf8");
		conn.on("connect", () => conn.write(`${JSON.stringify(message)}\n`));
		conn.on("data", (chunk: string) => {
			buf += chunk;
			const nl = buf.indexOf("\n");
			if (nl < 0) {
				if (buf.length > MAX_LINE) finish(new WireError("bad_reply", "reply too long"));
				return;
			}
			try {
				finish(null, JSON.parse(buf.slice(0, nl)));
			} catch {
				finish(new WireError("bad_reply", "reply is not JSON"));
			}
		});
		conn.on("end", () => {
			if (buf.trim()) {
				try {
					return finish(null, JSON.parse(buf));
				} catch {}
			}
			finish(new WireError("bad_reply", "connection closed without a reply"));
		});
		conn.on("error", (err: NodeJS.ErrnoException) => {
			finish(
				err.code === "ECONNREFUSED"
					? new WireError("refused", "connection refused")
					: new WireError("io", err.message),
			);
		});
	});
}

function tokenMatches(expected: string, given: unknown): boolean {
	if (typeof given !== "string") return false;
	const a = Buffer.from(expected);
	const b = Buffer.from(given);
	return a.length === b.length && timingSafeEqual(a, b);
}

/** Serve one-line requests. `handle` returns the reply object. */
function lineServer(handle: (req: any) => object): net.Server {
	return net.createServer((conn) => {
		let buf = Buffer.alloc(0);
		let done = false;
		const reply = (obj: object) => {
			if (done) return;
			done = true;
			conn.end(`${JSON.stringify(obj)}\n`);
		};
		const answer = (line: string) => {
			let req: any;
			try {
				req = JSON.parse(line);
			} catch {
				return reply({ ok: false, code: "bad_request", message: "the request is not JSON" });
			}
			if (!req || typeof req !== "object" || Array.isArray(req)) {
				return reply({ ok: false, code: "bad_request", message: "the request is not an object" });
			}
			try {
				reply(handle(req));
			} catch (err) {
				reply({ ok: false, code: "failed", message: err instanceof Error ? err.message : String(err) });
			}
		};
		conn.setTimeout(READ_TIMEOUT_MS, () => conn.destroy());
		conn.on("error", () => {});
		conn.on("data", (chunk: Buffer) => {
			if (done) return;
			buf = Buffer.concat([buf, chunk]);
			const nl = buf.indexOf(10);
			if (nl >= 0 && nl <= MAX_LINE) return answer(buf.subarray(0, nl).toString("utf8"));
			if (buf.length > MAX_LINE) reply({ ok: false, code: "bad_request", message: "request line over 1 MiB" });
		});
		// A client that closes its side without a newline still gets an answer.
		conn.on("end", () => {
			if (!done && buf.length > 0) answer(buf.toString("utf8"));
			else if (!done) conn.end();
		});
	});
}

// ---------------------------------------------------------------------------
// The extension

const NOTE_HINTS: Record<string, string> = {
	not_open: "that file is not the open document in plannotator-tui, nor a changed file in its code review",
	not_in_diff: "the code review does not show that line; pick a line of the new file inside a hunk, or leave line out for a note on the whole file",
	quote_not_found: "copy the quote exactly from the file's source text, or leave quote out",
	empty: "the note is empty",
	busy: "the person is typing a note in plannotator-tui; try again shortly",
	bad_request: "plannotator-tui did not accept the request",
	bad_token: "the plannotator-tui entry changed; try again",
};

export default function plannotatorTui(pi: ExtensionAPI) {
	let ctx: ExtensionContext | undefined;
	let server: net.Server | undefined;
	let entryPath: string | undefined;
	let token = "";
	let port = 0;
	let busy = false;
	let startingSince = 0;
	let sawSettled = false;
	let stopped = true;
	let warned = false;

	const warn = (message: string) => {
		if (warned) return;
		warned = true;
		try {
			ctx?.ui.notify(`plannotator-tui link: ${message}`, "warning");
		} catch {}
	};

	const sessionId = () => ctx?.sessionManager.getSessionId() ?? "";
	const sessionName = (): string | null => pi.getSessionName?.() ?? ctx?.sessionManager.getSessionName?.() ?? null;
	const idle = () => (ctx && typeof ctx.isIdle === "function" ? ctx.isIdle() : !busy);
	const isBusy = () => busy || !idle() || (startingSince > 0 && Date.now() - startingSince < STARTING_GRACE_MS);

	const writeEntry = () => {
		if (stopped || !ctx || !entryPath || !port) return;
		// `/tree` moves Pi's leaf without writing to the transcript, so plannotator-tui
		// would read the newest entry's branch. `tip` lets it tell whether `leaf` is still
		// current: once Pi appends, the newest entry is the leaf again.
		const entries = ctx.sessionManager.getEntries();
		try {
			writeAtomic(
				entryPath,
				`${JSON.stringify({
					v: VERSION,
					host: "pi",
					id: sessionId(),
					pid: process.pid,
					cwd: ctx.cwd,
					name: sessionName(),
					transcript: ctx.sessionManager.getSessionFile() ?? null,
					leaf: ctx.sessionManager.getLeafId() ?? null,
					tip: entries.at(-1)?.id ?? null,
					port,
					token,
					busy,
					updated: Date.now(),
				})}\n`,
			);
		} catch (err) {
			warn(`could not write ${entryPath}: ${err instanceof Error ? err.message : String(err)}`);
		}
	};

	const setBusy = (value: boolean) => {
		if (busy === value) return;
		busy = value;
		writeEntry();
	};

	// Last resort for an exit that skips session_shutdown. (Signals that kill the
	// process outright still leave the entry; readers check the pid.)
	const onExit = () => removeQuietly(entryPath);

	const stop = () => {
		stopped = true;
		server?.close();
		server = undefined;
		removeQuietly(entryPath);
		entryPath = undefined;
		port = 0;
		process.removeListener("exit", onExit);
	};

	const handleRequest = (req: any): object => {
		if (!tokenMatches(token, req.token)) return { ok: false, code: "bad_token", message: "wrong token" };
		switch (req.op) {
			case "ping":
				return { ok: true, busy: isBusy(), name: sessionName(), id: sessionId() };
			case "send": {
				const text = typeof req.text === "string" ? req.text : "";
				if (!text.trim()) return { ok: false, code: "empty", message: "text is empty" };
				try {
					if (!isBusy()) {
						// Mark busy before agent_start arrives, so a second send right away queues.
						startingSince = Date.now();
						pi.sendUserMessage(text);
						return { ok: true, disposition: "started" };
					}
					pi.sendUserMessage(text, { deliverAs: req.deliver === "steer" ? "steer" : "followUp" });
					return { ok: true, disposition: "queued" };
				} catch (err) {
					startingSince = 0;
					return { ok: false, code: "failed", message: err instanceof Error ? err.message : String(err) };
				}
			}
			default:
				return { ok: false, code: "bad_request", message: `unknown op ${JSON.stringify(req.op)}` };
		}
	};

	pi.on("session_start", async (_event, startCtx) => {
		stop(); // a second session_start in one instance replaces the old entry
		ctx = startCtx;
		stopped = false;
		busy = !idle();
		token = randomBytes(32).toString("hex");
		const root = registryRoot();
		entryPath = path.join(root, "pi", `${sessionId()}-${process.pid}.json`);
		const srv = lineServer(handleRequest);
		server = srv;
		try {
			await new Promise<void>((resolve, reject) => {
				srv.once("error", reject);
				srv.listen(0, "127.0.0.1", () => resolve());
			});
		} catch (err) {
			warn(`could not listen on 127.0.0.1: ${err instanceof Error ? err.message : String(err)}`);
			return;
		}
		if (server !== srv) return; // shut down while we were starting
		srv.on("error", () => {});
		port = (srv.address() as net.AddressInfo).port;
		process.on("exit", onExit);
		writeEntry();
	});

	pi.on("session_shutdown", async () => stop());

	pi.on("agent_start", async () => {
		startingSince = 0;
		setBusy(true);
	});

	// `agent_settled` (Pi 0.80.4+) marks the end of a run including retries and queued
	// follow-ups. On older Pi, fall back to `agent_end` once Pi reports idle.
	pi.on("agent_settled", async () => {
		sawSettled = true;
		setBusy(false);
	});
	pi.on("agent_end", async () => {
		if (sawSettled) return;
		// Don't hold up Pi's agent_end: check in the background.
		let tries = 0;
		const check = () => {
			if (sawSettled || stopped) return;
			if (idle() || ++tries >= 40) return setBusy(false);
			setTimeout(check, 50);
		};
		setTimeout(check, 50);
	});

	pi.on("session_info_changed", async () => writeEntry());
	pi.on("session_tree", async () => writeEntry());

	pi.registerTool({
		name: "plannotator_tui_note",
		label: "plannotator-tui note",
		description:
			"Leave a note in a running plannotator-tui (a terminal Markdown reviewer) that has the file open, so the person sees it beside the text. " +
			"`quote` must be copied exactly from the file's source text; the note attaches to its first occurrence, or the one nearest `line` when both are given. " +
			"With only `line`, the note goes on the block holding that line; with neither, on the first block. " +
			"plannotator-tui can also be reviewing a git diff (`plannotator-tui review`); then `file` is a changed file, `line` is a line of the new version that the diff shows, " +
			"and with neither `quote` nor `line` the note is on the whole file.",
		promptSnippet: "Leave a review note in the plannotator-tui window that has a file or a git diff open",
		parameters: Type.Object({
			file: Type.String({ description: "Path of the document, absolute or relative to the working directory" }),
			quote: Type.Optional(Type.String({ description: "Text copied exactly from the file, where the note attaches" })),
			line: Type.Optional(
				Type.Integer({
					minimum: 1,
					description: "1-based line in the file the note is about; with quote, picks the occurrence nearest this line",
				}),
			),
			note: Type.String({ description: "The note" }),
		}),
		async execute(_toolCallId, params, _signal, _onUpdate, toolCtx) {
			const cwd = toolCtx?.cwd ?? ctx?.cwd ?? process.cwd();
			const file = realOr(path.resolve(cwd, expandHome(params.file)));
			if (!params.note?.trim()) throw new Error("empty: the note is empty");
			const targets = tuiTargetsFor(file, liveTuiEntries());
			if (targets.length === 0) {
				throw new Error(`no_plannotator_tui: no running plannotator-tui has ${file} open; ask the person to open it there`);
			}
			const author = `pi · ${sessionName() || sessionId().slice(0, 8) || "session"}`;
			let last = "";
			for (const target of targets) {
				let reply: any;
				try {
					reply = await requestLine(target.port, {
						v: VERSION,
						token: target.token,
						op: "note",
						file,
						...(params.quote ? { quote: params.quote } : {}),
						...(Number.isInteger(params.line) && (params.line as number) > 0 ? { line: params.line } : {}),
						note: params.note,
						author,
					});
				} catch (err) {
					// A refused connection means a stale entry; try the next one.
					last = err instanceof WireError && err.code === "refused" ? "not_running" : `failed: ${(err as Error).message}`;
					continue;
				}
				if (reply?.ok === true) {
					return {
						content: [{ type: "text", text: `Note added to ${file} in plannotator-tui (pid ${target.pid}).` }],
						details: { file, pid: target.pid },
					};
				}
				const code = typeof reply?.code === "string" ? reply.code : "failed";
				last = `${code}: ${NOTE_HINTS[code] ?? reply?.message ?? "plannotator-tui refused the note"}`;
				// Another window may still have it open.
				if (code !== "not_open") break;
			}
			if (last === "not_running") last = `no_plannotator_tui: the plannotator-tui that had ${file} open is gone`;
			throw new Error(last);
		},
	});

	pi.registerCommand("plannotator-tui", {
		description: "List running plannotator-tui apps and their open files",
		handler: async (_args, cmdCtx) => {
			const root = registryRoot();
			const entries = liveTuiEntries(root);
			if (entries.length === 0) {
				cmdCtx.ui.notify(`No plannotator-tui is running (looked in ${path.join(root, "tui")}).`, "info");
				return;
			}
			const lines = [`plannotator-tui: ${entries.length} running`];
			for (const e of entries) {
				lines.push(`pid ${e.pid} · ${e.cwd}`);
				e.files.forEach((f, i) => lines.push(`  ${f}${i === 0 ? " (open)" : ""}`));
				if (e.files.length === 0) lines.push("  (no file open)");
			}
			cmdCtx.ui.notify(lines.join("\n"), "info");
		},
	});
}
