#!/usr/bin/env node
// End-to-end test of the plannotator-tui Pi extension against the real installed `pi`.
//
//   node integrations/pi/test.mjs          (KEEP=1 keeps the temp folder)
//
// Everything runs in a fresh folder under /var/tmp: HOME, Pi's agent and session dirs,
// the registry and XDG dirs all point there, so the real ~/.pi and ~/.plannotator are
// never read or written. The model is a fake OpenAI-compatible server in this process,
// and so is the "plannotator-tui" that receives notes.

import { spawn, spawnSync } from "node:child_process";
import * as fs from "node:fs";
import * as http from "node:http";
import * as net from "node:net";
import * as path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const EXTENSION = process.env.PI_LINK_EXTENSION ?? path.resolve(here, "../../crates/plannotator-tui/src/pi/plannotator-tui.ts");
const REPLY_DELAY_MS = 1500; // long enough to send while Pi is busy

// ---------------------------------------------------------------------------
// Bookkeeping, cleanup, assertions

const tmp = fs.mkdtempSync("/var/tmp/pi-link-test-");
const dirs = Object.fromEntries(
	["home", "agent", "sessions", "registry", "work", "xdg/config", "xdg/data", "xdg/state", "xdg/cache", "tmp"].map((d) => {
		const p = path.join(tmp, d);
		if (d !== "registry") fs.mkdirSync(p, { recursive: true }); // the extension creates the registry
		return [d, p];
	}),
);
const children = [];
const servers = [];
let failed = false;
let passed = 0;

function ok(cond, what, extra) {
	if (!cond) {
		failed = true;
		throw new Error(`FAIL ${what}${extra === undefined ? "" : `\n  got: ${typeof extra === "string" ? extra : JSON.stringify(extra)}`}`);
	}
	passed++;
	console.log(`ok   ${what}`);
}

function cleanup() {
	for (const c of children) {
		if (c.exitCode === null && c.signalCode === null) {
			try {
				c.kill("SIGKILL");
			} catch {}
		}
	}
	for (const s of servers) {
		try {
			s.closeAllConnections?.();
			s.close();
		} catch {}
	}
	if (!failed && !process.env.KEEP) fs.rmSync(tmp, { recursive: true, force: true });
	else console.log(`temp folder kept: ${tmp}`);
}
process.on("exit", cleanup);
for (const sig of ["SIGINT", "SIGTERM"]) process.on(sig, () => process.exit(130));

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
async function until(what, fn, timeoutMs = 15_000) {
	const end = Date.now() + timeoutMs;
	for (;;) {
		const v = await fn();
		if (v) return v;
		if (Date.now() > end) {
			failed = true;
			throw new Error(`FAIL timed out waiting for ${what}`);
		}
		await sleep(50);
	}
}

const listen = (srv) => new Promise((r) => srv.listen(0, "127.0.0.1", () => r(srv.address().port)));

// ---------------------------------------------------------------------------
// Fake model: OpenAI chat completions, streamed.
//   "CALL-NOTE {json}" in the last user message → a plannotator_tui_note tool call with those args.
//   a tool result as last message → records it and answers with text.
//   anything else → "ack N" after REPLY_DELAY_MS.

const llm = { requests: [], toolResults: [] };
const textOf = (c) => (typeof c === "string" ? c : Array.isArray(c) ? c.map((p) => p.text ?? "").join("") : "");
const llmServer = http.createServer((req, res) => {
	let body = "";
	req.on("data", (d) => (body += d));
	req.on("end", async () => {
		const j = JSON.parse(body || "{}");
		const msgs = j.messages ?? [];
		llm.requests.push(msgs);
		const last = msgs[msgs.length - 1] ?? {};
		const chunk = (delta, finish = null) =>
			`data: ${JSON.stringify({ id: "x", object: "chat.completion.chunk", created: 0, model: "fake", choices: [{ index: 0, delta, finish_reason: finish }] })}\n\n`;
		res.writeHead(200, { "content-type": "text/event-stream" });
		const lastText = textOf(last.content);
		const call = last.role === "user" ? lastText.match(/CALL-NOTE (\{.*\})/s) : null;
		if (last.role === "tool") {
			llm.toolResults.push(lastText);
			res.write(chunk({ role: "assistant", content: `tool said: ${lastText}` }));
			res.write(chunk({}, "stop"));
		} else if (call) {
			const n = llm.requests.length;
			res.write(
				chunk({
					role: "assistant",
					tool_calls: [{ index: 0, id: `call_${n}`, type: "function", function: { name: "plannotator_tui_note", arguments: call[1] } }],
				}),
			);
			res.write(chunk({}, "tool_calls"));
		} else {
			await sleep(REPLY_DELAY_MS);
			res.write(chunk({ role: "assistant", content: `ack ${llm.requests.length}` }));
			res.write(chunk({}, "stop"));
		}
		res.write("data: [DONE]\n\n");
		res.end();
	});
});
servers.push(llmServer);

// ---------------------------------------------------------------------------
// Fake plannotator-tui listener: records note requests and answers from a queue.

function fakeTui(token) {
	const tui = { token, got: [], answers: [] };
	tui.server = net.createServer((conn) => {
		let buf = "";
		conn.setEncoding("utf8");
		conn.on("data", (d) => {
			buf += d;
			const nl = buf.indexOf("\n");
			if (nl < 0) return;
			const req = JSON.parse(buf.slice(0, nl));
			tui.got.push(req);
			const reply = req.token !== token ? { ok: false, code: "bad_token" } : (tui.answers.shift() ?? { ok: true });
			conn.end(`${JSON.stringify(reply)}\n`);
		});
		conn.on("error", () => {});
	});
	servers.push(tui.server);
	return tui;
}

// ---------------------------------------------------------------------------
// Wire client: one JSON line out, one back. `raw` sends a string as is.

function wire(port, msg, { raw = false, halfClose = false } = {}) {
	return new Promise((resolve, reject) => {
		const c = net.connect({ host: "127.0.0.1", port });
		let buf = "";
		c.setEncoding("utf8");
		c.on("connect", () => {
			const line = raw ? msg : `${JSON.stringify(msg)}\n`;
			if (halfClose) c.end(line);
			else c.write(line);
		});
		c.on("data", (d) => {
			buf += d;
			const nl = buf.indexOf("\n");
			if (nl >= 0) {
				c.destroy();
				resolve(JSON.parse(buf.slice(0, nl)));
			}
		});
		c.on("error", (e) => (buf.includes("\n") ? null : reject(e)));
		c.on("close", () => reject(new Error(`closed without reply: ${buf}`)));
		setTimeout(() => {
			c.destroy();
			reject(new Error("wire timeout"));
		}, 10_000);
	});
}

// ---------------------------------------------------------------------------

async function main() {
	const llmPort = await listen(llmServer);
	fs.writeFileSync(
		path.join(dirs.agent, "models.json"),
		JSON.stringify({
			providers: {
				fake: {
					baseUrl: `http://127.0.0.1:${llmPort}/v1`,
					api: "openai-completions",
					apiKey: "x",
					models: [{ id: "fake", name: "fake", cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 } }],
				},
			},
		}),
	);
	const doc = path.join(dirs.work, "doc.md");
	fs.writeFileSync(doc, "# Title\n\nhello world\n");
	const docReal = fs.realpathSync(doc);

	const env = {
		PATH: process.env.PATH,
		LANG: process.env.LANG ?? "en_US.UTF-8",
		TERM: "dumb",
		HOME: dirs.home,
		TMPDIR: dirs.tmp,
		PI_CODING_AGENT_DIR: dirs.agent,
		PI_CODING_AGENT_SESSION_DIR: dirs.sessions,
		PLANNOTATOR_TUI_SESSIONS_DIR: dirs.registry,
		XDG_CONFIG_HOME: dirs["xdg/config"],
		XDG_DATA_HOME: dirs["xdg/data"],
		XDG_STATE_HOME: dirs["xdg/state"],
		XDG_CACHE_HOME: dirs["xdg/cache"],
		PI_OFFLINE: "1",
		PI_SKIP_VERSION_CHECK: "1",
		PI_TELEMETRY: "0",
	};
	// `/tree` needs a person at the TUI; this command does what it does, from RPC.
	const treeHelper = path.join(tmp, "tree-helper.ts");
	fs.writeFileSync(
		treeHelper,
		`export default function (pi) {
	pi.registerCommand("test-tree", { description: "navigate to an entry", handler: async (args, ctx) => { await ctx.navigateTree(args.trim(), { summarize: false }); } });
}
`,
	);
	const version = spawnSync("pi", ["--version"], { env, encoding: "utf8" });
	console.log(`pi ${(version.stdout || version.stderr).trim()} · temp ${tmp}`);

	const pi = spawn(
		"pi",
		["--mode", "rpc", "--provider", "fake", "--model", "fake", "--no-extensions", "--no-skills", "--no-context-files", "--offline", "--extension", EXTENSION, "--extension", treeHelper],
		{ cwd: dirs.work, env, stdio: ["pipe", "pipe", "pipe"] },
	);
	children.push(pi);
	const events = [];
	let stderr = "";
	let out = "";
	pi.stdout.setEncoding("utf8");
	pi.stdout.on("data", (d) => {
		out += d;
		let nl;
		while ((nl = out.indexOf("\n")) >= 0) {
			const line = out.slice(0, nl);
			out = out.slice(nl + 1);
			try {
				events.push(JSON.parse(line));
			} catch {}
		}
	});
	pi.stderr.on("data", (d) => (stderr += d));
	const exited = new Promise((r) => pi.on("exit", (code, sig) => r({ code, sig })));
	const rpc = (cmd) => pi.stdin.write(`${JSON.stringify(cmd)}\n`);
	const eventFrom = (start, pred) => () => events.slice(start).find(pred);

	// --- registration -------------------------------------------------------
	const piDir = path.join(dirs.registry, "pi");
	const entryFile = await until("the Pi entry", () => {
		try {
			const f = fs.readdirSync(piDir).find((n) => n.endsWith(".json") && !n.startsWith("."));
			return f && path.join(piDir, f);
		} catch {
			return undefined;
		}
	}, 30_000).catch((e) => {
		console.error(stderr);
		throw e;
	});
	const read = () => JSON.parse(fs.readFileSync(entryFile, "utf8"));
	let entry = read();
	ok(entry.v === 1 && entry.host === "pi", "entry has v:1 and host:pi", entry);
	ok(typeof entry.id === "string" && path.basename(entryFile) === `${entry.id}-${pi.pid}.json`, "entry file is <session id>-<pid>.json", path.basename(entryFile));
	ok(entry.pid === pi.pid, "entry pid is Pi's pid", entry.pid);
	ok([dirs.work, fs.realpathSync(dirs.work)].includes(entry.cwd), "entry cwd is the session cwd", entry.cwd);
	ok(entry.name === null, "entry name is null before a name is set", entry.name);
	const inSessions = (p) => typeof p === "string" && [dirs.sessions, fs.realpathSync(dirs.sessions)].some((d) => p.startsWith(`${d}/`));
	ok(entry.transcript === null || inSessions(entry.transcript), "entry transcript is null or in the isolated session dir", entry.transcript);
	ok(Number.isInteger(entry.port) && entry.port > 0, "entry has a port", entry.port);
	ok(/^[0-9a-f]{64}$/.test(entry.token), "entry token is 64 hex chars", entry.token);
	ok(entry.busy === false && typeof entry.updated === "number", "entry is idle with an updated time", entry);
	ok((fs.statSync(entryFile).mode & 0o777) === 0o600, "entry mode is 0600", (fs.statSync(entryFile).mode & 0o777).toString(8));
	ok((fs.statSync(piDir).mode & 0o777) === 0o700, "pi/ folder mode is 0700", (fs.statSync(piDir).mode & 0o777).toString(8));
	ok((fs.statSync(dirs.registry).mode & 0o777) === 0o700, "registry root, created by the extension, is mode 0700", (fs.statSync(dirs.registry).mode & 0o777).toString(8));
	const { port, token } = entry;

	// --- wire basics --------------------------------------------------------
	let r = await wire(port, { v: 1, token, op: "ping" });
	ok(r.ok === true && r.busy === false && r.id === entry.id && r.name === null, "ping answers ok, idle, with id and name", r);
	r = await wire(port, { v: 1, token: "0".repeat(64), op: "ping" });
	ok(r.ok === false && r.code === "bad_token", "ping with a wrong token → bad_token", r);
	r = await wire(port, { v: 1, op: "send", text: "x" });
	ok(r.ok === false && r.code === "bad_token", "send without a token → bad_token", r);
	r = await wire(port, { v: 1, token, op: "dance" });
	ok(r.ok === false && r.code === "bad_request", "unknown op → bad_request", r);
	r = await wire(port, "not json\n", { raw: true });
	ok(r.ok === false && r.code === "bad_request", "a line that is not JSON → bad_request", r);
	r = await wire(port, { v: 1, token, op: "send", text: "  \n " });
	ok(r.ok === false && r.code === "empty", "send with blank text → empty", r);
	r = await wire(port, JSON.stringify({ v: 1, token, op: "ping", extra: [1, 2] }), { raw: true, halfClose: true });
	ok(r.ok === true, "a request without a trailing newline (half-closed) is answered; unknown fields ignored", r);
	r = await wire(port, "x".repeat(1024 * 1024 + 10), { raw: true });
	ok(r.ok === false && r.code === "bad_request", "a request line over 1 MiB → bad_request", r);
	ok(llm.requests.length === 0, "none of that reached the model", llm.requests.length);

	// --- send: idle starts a turn, busy queues --------------------------------
	let mark = events.length;
	r = await wire(port, { v: 1, token, op: "send", text: "FEEDBACK-1 idle", deliver: "auto", source: "plannotator-tui" });
	ok(r.ok === true && r.disposition === "started", "send while idle → started", r);
	r = await wire(port, { v: 1, token, op: "send", text: "FEEDBACK-2 busy auto", source: "plannotator-tui" });
	ok(r.ok === true && r.disposition === "queued", "send right after (busy) → queued", r);
	r = await wire(port, { v: 1, token, op: "send", text: "FEEDBACK-3 busy steer", deliver: "steer", source: "plannotator-tui" });
	ok(r.ok === true && r.disposition === "queued", "send with deliver:steer while busy → queued", r);
	await until("entry busy:true", () => read().busy === true);
	ok(true, "entry was rewritten with busy:true during the turn");
	r = await wire(port, { v: 1, token, op: "ping" });
	ok(r.ok === true && r.busy === true, "ping reports busy during the turn", r);
	const queues = events.slice(mark).filter((e) => e.type === "queue_update");
	ok(queues.some((q) => q.followUp?.some((t) => t.includes("FEEDBACK-2"))), "FEEDBACK-2 went to Pi's follow-up queue", queues);
	ok(queues.some((q) => q.steering?.some((t) => t.includes("FEEDBACK-3"))), "FEEDBACK-3 went to Pi's steering queue", queues);
	await until("agent_settled", eventFrom(mark, (e) => e.type === "agent_settled"), 30_000);
	await until("entry busy:false", () => read().busy === false);
	ok(true, "entry went back to busy:false after agent_settled");
	const users = (llm.requests.at(-1) ?? []).filter((m) => m.role === "user").map((m) => textOf(m.content));
	ok(["FEEDBACK-1", "FEEDBACK-2", "FEEDBACK-3"].every((t) => users.some((u) => u.includes(t))), "the model saw all three messages", users);
	const transcriptPath = read().transcript;
	ok(typeof transcriptPath === "string" && fs.existsSync(transcriptPath), "entry transcript points at the session file", transcriptPath);
	const transcript = fs.readFileSync(transcriptPath, "utf8");
	ok(["FEEDBACK-1", "FEEDBACK-2", "FEEDBACK-3"].every((t) => transcript.includes(t)), "all three messages are in the transcript");

	// --- name ----------------------------------------------------------------
	rpc({ type: "set_session_name", name: "api-refactor" });
	await until("entry name", () => read().name === "api-refactor");
	ok(true, "set_session_name rewrites the entry name");
	r = await wire(port, { v: 1, token, op: "ping" });
	ok(r.name === "api-refactor", "ping reports the new name", r);

	// --- /tree: the entry says where Pi stands ---------------------------------
	const lines = () => fs.readFileSync(transcriptPath, "utf8").trim().split("\n").map((l) => JSON.parse(l));
	entry = read();
	ok(entry.leaf === lines().at(-1).id && entry.tip === entry.leaf, "entry leaf and tip are the transcript's newest entry", entry);
	const replies = lines().filter((l) => l.type === "message" && l.message.role === "assistant");
	const earlier = replies[0].id;
	ok(replies.length > 1, "the session has more than one reply to move between", replies.length);
	const before = lines();
	mark = events.length;
	rpc({ type: "prompt", message: `/test-tree ${earlier}` });
	await until("entry leaf after /tree", () => read().leaf === earlier);
	entry = read();
	ok(entry.tip === before.at(-1).id, "after /tree the entry's tip is still the newest entry on disk", entry);
	ok(lines().length === before.length, "/tree wrote nothing to the transcript", lines().length);
	rpc({ type: "prompt", message: "after the move" });
	await until("agent_settled after the move", eventFrom(mark, (e) => e.type === "agent_settled"), 30_000);
	await until("entry idle after the move", () => read().busy === false);
	entry = read();
	const after = lines();
	ok(entry.leaf === after.at(-1).id && entry.tip === entry.leaf, "after the next reply leaf and tip are its newest entry again", entry);
	const asked = after.find((l) => l.type === "message" && l.message.role === "user" && textOf(l.message.content).includes("after the move"));
	ok(asked?.parentId === earlier, "the next message went on the branch /tree moved to", asked);

	// --- note tool ------------------------------------------------------------
	const tuiDir = path.join(dirs.registry, "tui");
	fs.mkdirSync(tuiDir, { recursive: true, mode: 0o700 });
	const writeTui = (pid, p, tok, files, updated) =>
		fs.writeFileSync(
			path.join(tuiDir, `${pid}.json`),
			JSON.stringify({ v: 1, host: "plannotator-tui", pid, cwd: dirs.work, port: p, token: tok, files, updated }),
			{ mode: 0o600 },
		);
	// A: this test process, doc.md open. B: a newer app that only lists doc.md second.
	// C: a dead pid listing doc.md (stale; should be skipped and removed).
	const tuiA = fakeTui("a".repeat(64));
	const tuiB = fakeTui("b".repeat(64));
	const portA = await listen(tuiA.server);
	const portB = await listen(tuiB.server);
	const sleeper = spawn("sleep", ["120"], { stdio: "ignore" });
	children.push(sleeper);
	const dead = spawnSync(process.execPath, ["-e", "process.stdout.write(String(process.pid))"], { encoding: "utf8" });
	const deadPid = Number(dead.stdout);
	writeTui(process.pid, portA, tuiA.token, [docReal], Date.now() - 1000);
	writeTui(sleeper.pid, portB, tuiB.token, [path.join(fs.realpathSync(dirs.work), "other.md"), docReal], Date.now());
	writeTui(deadPid, 1, "c".repeat(64), [docReal], Date.now() + 1000);

	mark = events.length;
	rpc({ type: "prompt", message: `CALL-NOTE ${JSON.stringify({ file: "doc.md", quote: "hello world", line: 3, note: "looks good" })}` });
	await until("note at tui A", () => tuiA.got.length === 1);
	const note = tuiA.got[0];
	ok(
		note.v === 1 && note.op === "note" && note.token === tuiA.token && note.file === docReal && note.quote === "hello world" && note.line === 3 && note.note === "looks good",
		"the tool sent op:note with the real path, quote, line and note to the app whose open file matches",
		note,
	);
	ok(note.author === "pi · api-refactor", "note author is 'pi · <session name>'", note.author);
	ok(tuiB.got.length === 0, "the newer app that only lists the file second got nothing", tuiB.got);
	await until("tool result", () => llm.toolResults.length === 1);
	ok(/Note added to .*doc\.md/.test(llm.toolResults[0]), "the model got a short success result", llm.toolResults[0]);
	ok(!fs.existsSync(path.join(tuiDir, `${deadPid}.json`)), "the stale entry with a dead pid was removed");
	await until("agent_settled after tool", eventFrom(mark, (e) => e.type === "agent_settled"), 30_000);

	mark = events.length;
	tuiA.answers.push({ ok: false, code: "quote_not_found", message: "no such text" });
	rpc({ type: "prompt", message: `CALL-NOTE ${JSON.stringify({ file: docReal, quote: "nope", note: "n" })}` });
	await until("second tool result", () => llm.toolResults.length === 2);
	ok(tuiA.got.length === 2 && !("line" in tuiA.got[1]), "no line is sent when the model gives none", tuiA.got[1]);
	ok(/quote_not_found/.test(llm.toolResults[1]) && /exactly/.test(llm.toolResults[1]), "a quote_not_found answer is passed to the model with a hint", llm.toolResults[1]);
	const toolEnd = await until("tool_execution_end", eventFrom(mark, (e) => e.type === "tool_execution_end"));
	ok(toolEnd.isError === true, "that tool result is marked as an error", toolEnd);
	await until("agent_settled after tool 2", eventFrom(mark, (e) => e.type === "agent_settled"), 30_000);

	mark = events.length;
	rpc({ type: "prompt", message: `CALL-NOTE ${JSON.stringify({ file: "missing.md", note: "n" })}` });
	await until("third tool result", () => llm.toolResults.length === 3);
	ok(/no_plannotator_tui/.test(llm.toolResults[2]), "a file no app has open → no_plannotator_tui", llm.toolResults[2]);
	await until("agent_settled after tool 3", eventFrom(mark, (e) => e.type === "agent_settled"), 30_000);

	// --- command --------------------------------------------------------------
	mark = events.length;
	rpc({ type: "prompt", message: "/plannotator-tui" });
	const shown = await until("notify from /plannotator-tui", eventFrom(mark, (e) => e.type === "extension_ui_request" && e.method === "notify"));
	ok(
		shown.message.includes(`pid ${process.pid}`) && shown.message.includes(`pid ${sleeper.pid}`) && shown.message.includes(`${docReal} (open)`) && !shown.message.includes(`pid ${deadPid}`),
		"/plannotator-tui lists the live apps and their files",
		shown.message,
	);

	mark = events.length;
	rpc({ type: "prompt", message: "/plannotator-review" });
	const refused = await until("notify from /plannotator-review", eventFrom(mark, (e) => e.type === "extension_ui_request" && e.method === "notify"));
	ok(/needs Pi's terminal UI/.test(refused.message), "/plannotator-review runs nothing outside Pi's terminal UI", refused.message);

	// --- /new replaces the entry ----------------------------------------------
	rpc({ type: "new_session" });
	const newFile = await until("the new session's entry", () => {
		const f = fs.readdirSync(piDir).find((n) => n.endsWith(".json") && !n.startsWith(".") && path.join(piDir, n) !== entryFile);
		return f && !fs.existsSync(entryFile) && path.join(piDir, f);
	});
	const fresh = JSON.parse(fs.readFileSync(newFile, "utf8"));
	ok(fresh.id !== entry.id && fresh.pid === pi.pid && fresh.token !== token && fresh.name === null, "/new removes the old entry and writes one for the new session", fresh);
	ok(fs.readdirSync(piDir).filter((n) => n.endsWith(".json")).length === 1, "only one entry after /new", fs.readdirSync(piDir));
	await wire(port, { v: 1, token, op: "ping" }).then(
		(x) => ok(false, "the old listener is closed after /new", x),
		() => ok(true, "the old listener is closed after /new"),
	);
	r = await wire(fresh.port, { v: 1, token: fresh.token, op: "ping" });
	ok(r.ok === true && r.id === fresh.id, "the new listener answers ping with the new id", r);

	// --- shutdown -------------------------------------------------------------
	pi.stdin.end();
	const how = await Promise.race([exited, sleep(15_000).then(() => null)]);
	ok(how !== null, "Pi exits when stdin closes", how);
	ok(!fs.existsSync(newFile), "the entry is removed on shutdown");
	ok(fs.readdirSync(piDir).length === 0, "no temp files left in pi/", fs.readdirSync(piDir));
	ok(!stderr.includes("plannotator-tui link:"), "no extension warnings", stderr);
	const extErrors = events.filter((e) => e.type === "extension_error");
	ok(extErrors.length === 0, "no extension_error events", extErrors);
}

const guard = setTimeout(() => {
	failed = true;
	console.error("FAIL the whole test took over 120 s");
	process.exit(1);
}, 120_000);

main().then(
	() => {
		clearTimeout(guard);
		console.log(`\nPASS ${passed} checks`);
		process.exit(0);
	},
	(err) => {
		clearTimeout(guard);
		failed = true;
		console.error(`\n${err.message ?? err}`);
		process.exit(1);
	},
);
