/**
 * Three languages in one meeting, and three models listening for them.
 *
 * Summo could pair a specialist with the language named **first** and nothing with the rest, so a
 * meeting declared as Vietnamese, English and Japanese had a specialist for the Vietnamese and
 * whatever the multilingual model happened to hear for the other two. Asked for as *"tam ngữ cũng
 * được chứ cần gì song ngữ"*.
 *
 * `refine.rs` unit-tests the routing — which model an utterance goes to — and cannot test the half
 * that was actually broken: whether the models get **loaded at all**. That is four things agreeing
 * across three files — `pick_pair` returning a list, `resolve_models` putting the rest in
 * `also_refine`, `start_session` loading them, and `/status` naming them — and every one of them
 * was a single `Option` away from silently doing nothing, which is what it did.
 *
 * So this asserts the only thing that proves the feature exists: name three languages, and count
 * the speech models the daemon says it is running.
 *
 * No audio. Trilingual audio would make the suite about the fixture — whether a given clip is
 * recognised as Japanese — and the question here is about wiring. `bilingual.mjs` drives real
 * two-language audio and asserts transcripts; this one asserts that the third language is not
 * quietly dropped on the floor before any audio arrives.
 */
import { boot, plain } from "./daemon.mjs";
import { mirror } from "./mirror.mjs";

// One model that hears everything and labels each utterance, and one specialist per language.
// `whisper-base` rather than `tiny` for the same reason `bilingual.mjs` gives: the general model
// has to be the one that can tell the languages apart.
const GENERAL = "whisper-base";
const SPECIALISTS = {
  vi: "gipformer-1.5-68m",
  en: "parakeet-tdt-110m-en",
  ja: "sense-voice-small",
};
const MODELS = [GENERAL, ...Object.values(SPECIALISTS), "silero-vad-v5"];

const local = await mirror(MODELS, { name: "trilingual" });
if (local.unreachable.length > 0) {
  for (const { id, why } of local.unreachable) console.error(`${id}: ${why}`);
  console.error("this suite counts loaded models; it means nothing without all of them");
  process.exit(1);
}

const engine = await boot({ name: "trilingual", seed: false, registry: local.registry });
const at = (path) => `${engine.url}${path}${path.includes("?") ? "&" : "?"}token=${engine.token}`;
const problems = [];

async function install(id) {
  await fetch(at("/installs"), {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ id }),
  });
  for (let i = 0; i < 900; i += 1) {
    const jobs = await (await fetch(at("/installs"))).json();
    const job = jobs.find((candidate) => candidate.model === id);
    if (job?.state === "done") return;
    if (job?.state === "failed") throw new Error(`${id}: ${job.error ?? "install failed"}`);
    await new Promise((resolve) => setTimeout(resolve, 500));
  }
  throw new Error(`${id}: still installing after 450s`);
}

for (const id of MODELS) {
  console.log(`installing ${id}…`);
  await install(id);
}

/**
 * Both answers, because they are different claims.
 *
 * `/status` says what the *session* resolved to — the daemon's own reading of "which models is
 * this meeting configured with". `/perf` says what is *loaded*, which is the reading that costs
 * memory and the one a "why is this using 900 MB" question is answered from. A feature that
 * resolves three models and loads one would pass a test that only asked the first.
 */
const readings = async () => ({
  status: await (await fetch(at("/status"))).json(),
  perf: await (await fetch(at("/perf"))).json(),
});

/** Start a meeting the way the app does — over the socket, naming languages and no model. */
async function record(languages) {
  const socket = new WebSocket(`ws://127.0.0.1:${engine.port}/ws?token=${engine.token}`);
  await new Promise((resolve, reject) => {
    socket.addEventListener("open", resolve, { once: true });
    socket.addEventListener("error", reject, { once: true });
  });
  socket.send(JSON.stringify({ cmd: "session_start", languages, lanes: ["mic"] }));
  // Loading three decoders is several seconds; poll rather than guess at it.
  for (let i = 0; i < 120; i += 1) {
    const status = await (await fetch(at("/status"))).json();
    if (status.state === "recording") {
      // The refiners are loaded after the session is announced, so give the last one a moment to
      // land before counting. A count taken too early is the bug this suite exists to catch,
      // reported as a pass.
      await new Promise((resolve) => setTimeout(resolve, 2500));
      return { socket, ...(await readings()) };
    }
    await new Promise((resolve) => setTimeout(resolve, 500));
  }
  console.error("status:", JSON.stringify(await (await fetch(at("/status"))).json()));
  throw new Error("the daemon never reported a recording");
}

async function stop(socket) {
  socket.send(JSON.stringify({ cmd: "session_stop" }));
  await new Promise((resolve) => setTimeout(resolve, 1200));
  socket.close();
}

// ---- three languages, three models ----------------------------------------
{
  const { socket, status, perf } = await record(["vi", "en", "ja"]);
  const resolved = [status.refine_model, ...(status.also_refine ?? [])].filter(Boolean);
  const loaded = (perf.models ?? []).filter((m) => m.role === "refine").map((m) => m.id);
  console.log(`live: ${status.live_model}`);
  console.log(`resolved: ${resolved.join(", ") || "none"}`);
  console.log(`loaded: ${loaded.join(", ") || "none"}`);

  if (status.live_model !== GENERAL) {
    problems.push(
      `the model that can tell the languages apart is not the one listening: ${status.live_model}`,
    );
  }
  // The point. Before this, exactly one specialist was chosen whatever was named.
  for (const [code, id] of Object.entries(SPECIALISTS)) {
    if (!resolved.includes(id)) {
      problems.push(`${code} was named and ${id} was not chosen: ${resolved.join(", ")}`);
    }
    // And chosen is not loaded. A list that resolves correctly and loads one model is the same
    // failure wearing a longer name.
    if (!loaded.includes(id)) {
      problems.push(`${id} was chosen for ${code} and is not running: ${loaded.join(", ")}`);
    }
  }
  // And the log says which models are doing the refining, which is where a support question about
  // a language coming back wrong is answered from.
  const said = plain(engine).match(/refining with.*/);
  console.log(said ? said[0].trim() : "the daemon did not say what it is refining with");
  if (!said) problems.push("nothing in the log names the models doing the refining");

  await stop(socket);
}

// ---- one language, one model ----------------------------------------------
//
// The other half of the claim, and the one a change here is most likely to break: naming one
// language must not drag three decoders into memory. Each is several hundred megabytes.
{
  const { socket, perf } = await record(["vi"]);
  const loaded = (perf.models ?? []).filter((m) => m.role === "refine").map((m) => m.id);
  console.log(`one language named, refine: ${loaded.join(", ") || "none"}`);
  // Exactly one, not "no more than one": zero would mean the pairing this feature was built on
  // top of had quietly stopped working, and a test that only forbids too many would call that a
  // pass. What it should be is the general model, catching the sentences the specialist cannot
  // hear — see `automatic_second`.
  if (loaded.length !== 1) {
    problems.push(`one language named and ${loaded.length} second models loaded: ${loaded}`);
  } else if (loaded[0] !== GENERAL) {
    problems.push(
      `the second model for a single-language meeting is not the general one: ${loaded}`,
    );
  }
  await stop(socket);
}

engine.stop();

if (problems.length > 0) {
  console.error(`\n${problems.length} problem(s):`);
  for (const p of problems) console.error(`  - ${p}`);
  process.exit(1);
}
console.log("\ntrilingual ok");
