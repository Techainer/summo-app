# Browser end-to-end suites

Six suites, each driving a real daemon and a real vault on disk. They exist to catch the class of
bug unit tests structurally cannot: a route that is not registered, a field the app reads under a
different name than the daemon writes, a control that renders but cannot be clicked.

```bash
# 1. Build and serve the interface
pnpm --filter @summo/web build
python3 -m http.server 8903 -d apps/web/dist &

# 2. Start a daemon over a seeded vault. --dev lets a page served from this machine reach it;
#    a shipped build refuses that, which is what stops a website reaching your microphone.
SUMMO_HOME=/tmp/summo-e2e cargo run -p summo-engine -- --port 8712 --dev &

# 3. Drive it
TOKEN=$(python3 -c "import json;print(json.load(open('/tmp/summo-e2e/engine.json'))['token'])")
cd apps/web
node e2e/shell.mjs   http://127.0.0.1:8903/index.html 8712 "$TOKEN"
node e2e/meeting.mjs http://127.0.0.1:8903/index.html 8712 "$TOKEN"
node e2e/tasks.mjs   http://127.0.0.1:8903/index.html 8712 "$TOKEN"
node e2e/chat.mjs    http://127.0.0.1:8903/index.html 8712 "$TOKEN"
node e2e/nudges.mjs  http://127.0.0.1:8903/index.html 8712 "$TOKEN" /tmp/summo-e2e/nudges.json
node e2e/draft.mjs   http://127.0.0.1:8903/index.html 8712 "$TOKEN" 01A1 /tmp/summo-e2e/vault/meetings/m1.md
```

| Suite | What only a browser shows |
|---|---|
| `shell` | every screen renders, the sidebar collapses out of the accessibility tree, the bundled fonts load and Vietnamese diacritics stay inside their line box |
| `meeting` | the audio route is reachable with the lane name the client derives, the transcript virtualises into clickable rows |
| `tasks` | filtering narrows a column, the agent's board is separate and its step list expands |
| `chat` | a failing request reaches the user instead of showing nothing |
| `nudges` | a nudge reaches the screen and dismissing clears it |
| `draft` | an unapproved section appears once, selecting a passage offers to refine it, confirming keeps the text |

## Two suites take extra arguments, and the reason is the same

`draft` and `nudges` **consume the thing they test**. Confirming a draft removes the marker;
asking for a nudge is what records it as said. Both therefore seed or reset their own state, and
need to be told where it lives. Without that they pass once and then report an empty screen
forever — which reads as a regression and is not one.

Screenshots land in `/tmp/shots/`.
