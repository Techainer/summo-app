/**
 * A registry whose files come from this machine.
 *
 * The catalogue suite installs two models and then removes them, which is the only way to check
 * that installing and removing are real. Those installs used to fetch from the addresses in the
 * published manifests — `github.com/snakers4/…` — and that made a browser test depend on a third
 * party being reachable and willing. It was, until eleven suites ran in a row and it answered
 * `error sending request`, at which point `pnpm e2e` failed on a screen that had nothing wrong with
 * it.
 *
 * So: copy the manifests and rewrite the file URLs of the named models to `file://` addresses in a
 * cache under `/tmp`. The cache is filled once — from a blob some vault on this machine already
 * has, or failing that from the real address — and reused afterwards, so a machine that has run
 * this before needs no network at all.
 *
 * `file://` rather than a local HTTP server because `manifest.rs` accepts only `https://` and
 * `file://`, and that rule is worth keeping: a registry that can point a download at plain HTTP is
 * a registry that can be told to by whoever controls the network. A model served over `http://`
 * simply disappears from the catalogue, which is how the first version of this file failed.
 *
 * The checksums are left exactly as published. Serving different bytes under the same manifest
 * would make the suite pass on a mirror it should reject — the point is to move where the bytes
 * come from, not to stop checking them.
 */
import { createHash } from "node:crypto";
import { cpSync, existsSync, mkdirSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
/**
 * The registry checkout.
 *
 * Beside this repository by default, which is where a developer clones it. On CI it cannot be:
 * `actions/checkout` refuses a path outside the workspace, so the job clones it *inside* and points
 * here with `SUMMO_REGISTRY_DIR`. The old line was a fixed `../../../../summo-registry` and the
 * browser job spent its life failing on "Repository path is not under GITHUB_WORKSPACE".
 */
const REGISTRY = process.env.SUMMO_REGISTRY_DIR ?? join(HERE, "../../../../summo-registry");

/** Shared between runs on purpose: re-downloading the same bytes every suite is what is being fixed. */
const CACHE = "/tmp/summo-e2e-model-cache";

/**
 * Put one file in the cache and return where it landed.
 *
 * Named by checksum rather than by file name, so two manifests that disagree about what
 * `model.onnx` contains cannot share an entry.
 */
async function cache(url, sha256) {
  mkdirSync(CACHE, { recursive: true });
  const at = join(CACHE, sha256);
  if (existsSync(at)) return at;

  // Anything already installed on this machine will do. Summo's store is content-addressed, so a
  // blob found under any home is by definition the bytes this manifest asks for — and it is checked
  // below regardless of where it came from.
  const local = fromLocalStores(sha256);
  const bytes = local ?? Buffer.from(await download(url));

  const got = createHash("sha256").update(bytes).digest("hex");
  if (got !== sha256) {
    throw new Error(
      `mirror: ${local ? "a local blob" : url} does not match its published checksum`,
    );
  }

  writeFileSync(at, bytes);
  return at;
}

/**
 * The address to actually ask, and what to send with it.
 *
 * The manifests publish `github.com/owner/repo/raw/ref/path`, which is a *redirect* to
 * `raw.githubusercontent.com`. Unauthenticated, from a shared address, github.com refuses that
 * redirect — and it refuses it as `404`, which is indistinguishable from a file that moved. That is
 * what failed the browser job here: the file was never gone. Asked directly of the content host,
 * with the token every Actions run already has, the same request is served.
 *
 * The token is sent to GitHub's own hosts and nowhere else. A manifest may point at any address it
 * likes — that is the whole point of a registry anyone can publish to — and a credential must not
 * follow it there.
 */
function request(url) {
  const at = new URL(url);
  const raw = at.hostname === "github.com" ? at.pathname.replace("/raw/", "/") : null;
  const asked = raw ? new URL(`https://raw.githubusercontent.com${raw}${at.search}`) : at;

  const token = process.env.GITHUB_TOKEN ?? process.env.GH_TOKEN;
  const github = asked.hostname === "raw.githubusercontent.com" || asked.hostname === "github.com";
  return {
    url: asked.toString(),
    headers: token && github ? { authorization: `Bearer ${token}` } : {},
  };
}

/**
 * Fetch the bytes, allowing for the fact that the other end is a stranger.
 *
 * The cache above means this runs at most once per model per machine — but "at most once" is still
 * once, and on a fresh CI runner that once is an unauthenticated request to github.com. It answers
 * `429` when several jobs share an address, and it has answered `404` for the same URL that works a
 * minute later. Either killed the whole browser run, on a suite about installing models, over a
 * screen with nothing wrong with it. Which is the failure this file was written to stop; it just
 * stopped it for the tenth download and not for the first.
 *
 * So: retried, with a widening gap. Anything in the 400s that is not `429` is left alone — a
 * genuinely wrong URL should fail on the first attempt and say so, rather than three times slowly.
 */
async function download(url, allowed = 4) {
  const asked = request(url);
  let last = "";
  let tried = 0;
  for (let attempt = 0; attempt < allowed; attempt += 1) {
    if (attempt > 0) await new Promise((r) => setTimeout(r, 2000 * 2 ** (attempt - 1)));
    tried += 1;
    try {
      const response = await fetch(asked.url, { headers: asked.headers });
      if (response.ok) return await response.arrayBuffer();
      last = `answered ${response.status}`;
      const retryable = response.status === 429 || response.status >= 500;
      // `404` from GitHub raw under an abuse limit is indistinguishable from a moved file, so it is
      // retried once and then believed.
      if (!retryable && !(response.status === 404 && attempt === 0)) break;
    } catch (e) {
      last = `could not be reached (${e.message})`;
    }
  }
  // The count that happened, not the count allowed. A message saying "after 4 attempts" for a URL
  // asked twice sends whoever reads it looking for three minutes of retries that never ran.
  //
  // The address that was actually asked, which is not always the one in the manifest.
  throw new Error(
    `mirror: ${asked.url} ${last} after ${tried} ${tried === 1 ? "attempt" : "attempts"}`,
  );
}

/**
 * The same blob, if some vault on this machine already has it.
 *
 * `models/blobs/sha256/ab/abcd…` under any Summo home. Checked before the network because anybody
 * who has run the app has these bytes already, and a test that downloads a gigabyte it could have
 * read from disk is a test people learn to skip.
 */
function fromLocalStores(sha256) {
  const shard = sha256.slice(0, 2);
  for (const root of ["/tmp", process.env.HOME].filter(Boolean)) {
    let entries = [];
    try {
      entries = readdirSync(root);
    } catch {
      continue;
    }
    for (const entry of entries) {
      const at = join(root, entry, "models/blobs/sha256", shard, sha256);
      try {
        if (existsSync(at)) return readFileSync(at);
      } catch {
        // A home being torn down by another suite. Try the next one.
      }
    }
  }
  return null;
}

/**
 * Mirror `ids` locally and return `{ registry, stop }`.
 *
 * Only the named models are touched. Everything else keeps its published URL, so a suite that
 * installs something unexpected fails loudly rather than quietly reaching the internet.
 */
export async function mirror(ids, { name = "mirror" } = {}) {
  const root = join("/tmp", `summo-registry-${name}-${process.pid}`);
  cpSync(REGISTRY, root, { recursive: true });

  const wanted = new Set(ids);
  const seen = new Set();
  /** Models whose bytes could not be fetched at all. See the note on the return value. */
  const unreachable = [];

  for (const file of readdirSync(join(root, "models"))) {
    const at = join(root, "models", file);
    const manifest = JSON.parse(readFileSync(at, "utf8"));
    if (!wanted.has(manifest.id)) continue;
    seen.add(manifest.id);

    try {
      for (const entry of manifest.files ?? []) {
        entry.url = pathToFileURL(await cache(entry.url, entry.sha256)).href;
      }
      writeFileSync(at, JSON.stringify(manifest, null, 2));
    } catch (e) {
      // Only a fetch that failed. A checksum that does not match is still fatal — that is the
      // check being made, not an accident of the network.
      if (!e.message.startsWith("mirror: http")) throw e;
      unreachable.push({ id: manifest.id, why: e.message });
    }
  }

  // A model that is not in the registry would otherwise be mirrored silently and then fail in the
  // browser as "the card is not there", which is a long way from the cause.
  for (const id of wanted) {
    if (!seen.has(id)) throw new Error(`mirror: ${id} is not in ${REGISTRY}`);
  }

  /**
   * `unreachable` is the difference between "this failed" and "this could not be run".
   *
   * The cache above means the bytes are fetched once per machine, and the CI job keeps that cache
   * between runs — so this list is empty on every run but the first after a registry change. When
   * it is not empty, github.com has refused a runner rather than the app having done anything
   * wrong, and the suite reporting a red build for a screen with nothing wrong with it is the
   * exact failure this file exists to prevent. It just prevented it for the tenth download and not
   * the first.
   *
   * Reported, never silent: a caller that ignores this and installs anyway gets a card that will
   * not install, and a reader of the log has to be told which check did not happen and why.
   */
  return { registry: root, unreachable, stop: async () => {} };
}
