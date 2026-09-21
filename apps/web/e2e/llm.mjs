/**
 * A language model that is a local HTTP server.
 *
 * Every browser suite that touches a summary has, until now, worked around not having one.
 * `draft.mjs` writes `<!-- summo:draft -->` into the meeting file by hand and tests the screen
 * around it, because nothing in the suite could produce a draft — so the one control on the meeting
 * page that reaches a model had never been pressed by a test.
 *
 * Deliberately a real socket rather than a route interception. The daemon makes this request, not
 * the browser: `page.route` cannot see it, and a suite that stubbed it in the browser would be
 * testing a conversation the product does not have.
 *
 * Only what `summo_llm`'s OpenAI wire needs: a chat-completions endpoint and a models list for the
 * connection test. Not a simulation of anybody's API — `crates/summo-engine/tests/` covers the
 * wire, and what this exists for is the button.
 */
import { createServer } from "node:http";
import { writeFileSync, mkdirSync } from "node:fs";
import { join } from "node:path";

/**
 * Serve a model that answers every completion with `reply`.
 *
 * Returns `{ url, asked, stop }`, where `asked` is every request body it has seen — a suite that
 * only checks the screen cannot tell a prompt built from an empty transcript from a good one, and
 * both produce a plausible-looking draft.
 */
export async function model(reply) {
  const asked = [];

  const server = createServer((req, res) => {
    let body = "";
    req.on("data", (chunk) => (body += chunk));
    req.on("end", () => {
      if (req.url?.includes("/models")) {
        res.writeHead(200, { "content-type": "application/json" });
        res.end(JSON.stringify({ data: [{ id: "stub", object: "model" }] }));
        return;
      }
      asked.push(body);
      res.writeHead(200, { "content-type": "application/json" });
      res.end(
        JSON.stringify({
          choices: [{ message: { role: "assistant", content: reply } }],
        }),
      );
    });
  });

  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  const { port } = server.address();

  return {
    url: `http://127.0.0.1:${port}/v1`,
    asked,
    stop: () => new Promise((resolve) => server.close(resolve)),
  };
}

/**
 * Point a daemon's home at `url` as its language model.
 *
 * Written as an extra provider rather than by overriding a preset, because that is the path a user
 * takes: `providers.json` is the documented way to add an endpoint, and `summo_llm::provider`'s
 * catalogue merges it over the built-ins. A suite that reached for a preset would be exercising a
 * code path the product only uses for OpenAI and Anthropic.
 */
export function useModel(home, url) {
  mkdirSync(home, { recursive: true });
  writeFileSync(
    join(home, "providers.json"),
    JSON.stringify(
      [{ id: "stub", name: "Stub", base_url: url, model: "stub", wire: "open-ai" }],
      null,
      2,
    ),
  );
  writeFileSync(
    join(home, "settings.json"),
    JSON.stringify({ llm: { provider: "stub", model: "stub" } }, null, 2),
  );
}
