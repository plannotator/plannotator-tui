// `/plannotator-review`'s runner, without Pi: a fake TUI records stop/start, a fake process
// stands in for Pi's (its signals are emitted, not sent), and /bin/sh plays the review.
//
//   node --experimental-transform-types integrations/pi/run-in-place.test.mjs
//
// Needs Node 22.7 or later (the extension has a parameter property, which plain type
// stripping refuses). The extension's only runtime import outside Node, `typebox`, is
// stubbed. `cargo test` runs this when such a Node is on PATH.

import assert from "node:assert/strict";
import { EventEmitter } from "node:events";
import { register } from "node:module";
import * as path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const EXTENSION = process.env.PI_LINK_EXTENSION ?? path.resolve(here, "../../crates/plannotator-tui/src/pi/plannotator-tui.ts");

register(
	"data:text/javascript," +
		encodeURIComponent(`
export async function resolve(specifier, context, next) {
  if (specifier === "typebox") {
    return { url: "data:text/javascript,export const Type = new Proxy({}, { get: () => () => ({}) });", shortCircuit: true };
  }
  return next(specifier, context);
}`),
);
const { runInPlace, ranNotice } = await import(pathToFileURL(EXTENSION).href);

function fakeTui() {
	const calls = [];
	return { calls, stop: () => calls.push("stop"), start: () => calls.push("start"), requestRender: () => calls.push("render") };
}

/** A stand-in for Pi's process: signals are emitted at it, and a re-raise is recorded. */
function fakeProcess() {
	const proc = new EventEmitter();
	proc.pid = 4242;
	proc.raised = [];
	proc.kill = (pid, signal) => proc.raised.push([pid, signal]);
	return proc;
}

const sh = (script) => ["/bin/sh", ["-c", script]];
let passed = 0;
async function check(name, body) {
	await body();
	passed += 1;
	console.log(`ok - ${name}`);
}

await check("Pi's screen steps aside before the program and comes back after it", async () => {
	const tui = fakeTui();
	const proc = fakeProcess();
	const [bin, args] = sh("exit 3");
	const ran = await runInPlace(tui, bin, args, process.cwd(), process.env, proc);
	assert.equal(ran.code, 3);
	assert.deepEqual(tui.calls, ["stop", "start", "render"]);
	assert.equal(ranNotice("plannotator-tui", ran), "plannotator-tui exited with status 3");
	for (const signal of ["SIGINT", "SIGTERM", "SIGHUP", "exit"]) assert.equal(proc.listenerCount(signal), 0, signal);
});

await check("a program ended by a signal says which", async () => {
	const tui = fakeTui();
	const [bin, args] = sh("kill -KILL $$");
	const ran = await runInPlace(tui, bin, args, process.cwd(), process.env, fakeProcess());
	assert.equal(ran.code, null);
	assert.equal(ran.signal, "SIGKILL");
	assert.equal(ranNotice("plannotator-tui", ran), "plannotator-tui was stopped by signal SIGKILL");
	assert.deepEqual(tui.calls, ["stop", "start", "render"]);
});

await check("a program that cannot start leaves Pi's screen as it was", async () => {
	const tui = fakeTui();
	const ran = await runInPlace(tui, "/nonexistent/plannotator-tui", [], process.cwd(), process.env, fakeProcess());
	assert.ok(ran.error, "says why");
	assert.match(ranNotice("/nonexistent/plannotator-tui", ran), /^Could not run/);
	assert.deepEqual(tui.calls, ["stop", "start", "render"]);
});

await check("Pi told to end, with handlers of its own: the program ends, the screen comes back first", async () => {
	const tui = fakeTui();
	const proc = fakeProcess();
	const order = [];
	proc.on("SIGTERM", () => order.push(`pi saw SIGTERM after ${tui.calls.join(",")}`));
	const [bin, args] = sh("sleep 30");
	const running = runInPlace(tui, bin, args, process.cwd(), process.env, proc);
	await new Promise((r) => setTimeout(r, 200));
	proc.emit("SIGTERM", "SIGTERM");
	const ran = await running;
	assert.equal(ran.signal, "SIGTERM", "the program was ended");
	assert.deepEqual(order, ["pi saw SIGTERM after stop,start,render"], "ours ran first");
	assert.deepEqual(tui.calls, ["stop", "start", "render"], "restored once");
	assert.deepEqual(proc.raised, [], "Pi's own handler decides");
	assert.equal(proc.listenerCount("SIGTERM"), 1, "only Pi's own is left");
});

await check("Pi told to end, with no handler: the program ends and Pi ends as it would have", async () => {
	const tui = fakeTui();
	const proc = fakeProcess();
	const [bin, args] = sh("sleep 30");
	const running = runInPlace(tui, bin, args, process.cwd(), process.env, proc);
	await new Promise((r) => setTimeout(r, 200));
	proc.emit("SIGHUP", "SIGHUP");
	const ran = await running;
	assert.equal(ran.signal, "SIGTERM");
	assert.deepEqual(proc.raised, [[4242, "SIGHUP"]], "the signal is raised again with ours gone");
	assert.deepEqual(tui.calls, ["stop"], "the screen is left put away for the shell");
	assert.equal(proc.listenerCount("SIGHUP"), 0);
});

console.log(`${passed} checks passed`);
