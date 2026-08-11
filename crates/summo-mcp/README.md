# summo-mcp

The vault, as tools an assistant can call. Point Claude Code, Cursor or any MCP client at this
binary and questions about what was said in a meeting are answered from the user's own transcripts
instead of guessed at.

```json
{
  "mcpServers": {
    "summo": {
      "command": "summo-mcp",
      "env": { "SUMMO_HOME": "/home/you/.summo" }
    }
  }
}
```

`SUMMO_HOME` is optional; without it the same discovery the app uses applies.

## Tools

| | |
|---|---|
| `search_meetings` | Excerpts across every transcript, with meeting ids and timestamps. Vietnamese matches with or without diacritics. |
| `get_meeting` | One meeting in full: summary, sections, whole transcript. |
| `list_meetings` | Recent meetings — dates, titles, folders. |
| `list_tasks` | Open tasks with owners and due dates. |

## Read-only, on purpose

Nothing here writes. No tool creates a task, edits a note or starts a recording. An MCP client is a
model holding a tool list, and a model that misreads an instruction should not be able to rewrite
somebody's meeting notes — writing stays behind a person pressing a button in the app.

A test enforces it: any tool named with a writing verb fails the suite.

## No daemon, no port, no token

The vault is Markdown on disk and this reads those files directly. So it works with the app closed,
and there is no second copy of the auth story to get wrong.

Logging goes to stderr. One JSON object per line on stdout is the framing every MCP stdio client
expects, and a stray log line there is a parse error the client reports as the server being broken.
