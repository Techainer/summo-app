# 4. The agent loop comes from aionrs

Date: 2026-08-10

## Status

Accepted.

## Context

Summo needs an agent: something that can take "tạo lịch cho mốc ra mắt", work out the steps, call
tools, recover from a model returning malformed JSON, keep a session across restarts, and talk to
MCP servers so the user's Jira and Slack are reachable.

None of that is about meetings. All of it is hard, and all of it is a moving target — tool-calling
protocols, prompt caching, context compaction and MCP transports have all changed shape more than
once in the last two years.

## Decision

**Use `iOfficeAI/aionrs` (Apache-2.0) as the agent core. Do not write an agent loop.**

`crates/summo-agent` contains only what is specific to Summo:

* five tools about meetings, implementing `aion_tools::Tool`
* `steps.rs`, which writes the agent's own plan back into the vault

Everything else — the streaming loop, tool dispatch, retries, malformed-turn handling, context
compaction, session persistence, the provider layer and the MCP client — is upstream's.

### Corrections to the original plan

The plan said aionrs was "CLI-focused" and would need a `lib.rs` added. That was wrong: `aion-agent`
already exposes one, publishing `engine`, `session`, `turn`, `orchestration` and `tool_policy`. The
integration is a dependency, not a fork.

The plan also said to vendor the crates into `crates/summo-agent-core/`. This uses a **git dependency
pinned to a commit** instead. Vendoring 57,000 lines would mean merging upstream's workspace
dependency table into ours and reviewing a commit nobody can meaningfully review. A pinned revision
is reproducible today, and `cargo vendor` produces an offline tree at release time, which is the
property vendoring was wanted for. Revisit if we ever need to patch upstream rather than extend it.

Apache-2.0 is compatible one way with AGPL-3.0, so this composes and can be sold. `NOTICE` carries
the attribution.

## Consequences

* Upgrading is a revision bump plus a test run, not a merge.
* Upstream decides what the loop does. If that becomes a problem the fork is still available; the
  pinned revision means it can be taken at a known-good point.
* Anthropic, OpenAI-compatible, Bedrock and Vertex all arrive for free through `aion-providers`,
  which duplicates part of `summo-llm`. `summo-llm` stays for now: it serves summarise and translate,
  which are one-shot requests with no agent involved, and collapsing them would make a summary
  depend on the agent stack.
* The tool set is deliberately narrow and has no shell. An agent that can run commands can do
  anything, which means the user cannot predict it and therefore cannot consent to it. Every tool is
  something a user would recognise as a thing Summo already does.
* Tools take meeting and task ids, never paths, so a hallucinated `../../.ssh/id_rsa` resolves to
  "no meeting with that id". Tested directly.
