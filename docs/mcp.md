# MCP conventions

Start explicitly with `pdfcraft-cli mcp [--root DIR] [--compact]`. The server uses JSON lines on
stdin/stdout, opens no port, and stops on EOF. `--root` confines all reads and writes, including
commands invoked through the common tools. Logs stay on stderr.

Supply `--root` at most once, followed by a nonempty directory path. Missing operands,
repeated roots and another long option in place of the directory are refused before any
script is read or server is started. Prefix a directory name beginning with `--` with `./`.

| Tool | Arguments |
|---|---|
| `command_list` | `doc?`, `filter?`, `enabled_only?`; returns `{commands: [...]}` with mapped tool schemas |
| `command_run` | `id`, `params?` (the mapped task tool's arguments, including `doc` when required) |
| `command_batch` | `steps: [{id, params?}]`, `stop_on_error?` (default true) |
| `doc_inspect` | `doc?`; one document's info or `{documents: [...]}` for the session |
| `render_preview` | `doc?`, `page?` (1-based, default 1), `max_side?` (1–4096, default 1024) |

All existing task tools remain available. Compact mode lists these five tools alongside its
existing core set and `tool_search` / `tool_call`. There are no legacy-name aliases. The full
[automation guide](../crates/automation/README.md) describes the tools, resources, coordinate
system, root confinement, and errors.

```text
command_run   {"id":"file.open","params":{"path":"input.pdf"}}
              → {"doc":1,...}
command_list  {"doc":1,"filter":"rotate"}
command_run   {"id":"page.rotate","params":{"doc":1,"degrees":90}}
render_preview {"doc":1,"page":1,"max_side":512}
```

Every tool has a title and the four MCP annotation hints. Unknown top-level argument keys return
JSON-RPC `-32602`; command parameter keys that do not belong to the mapped task tool are ignored
with warnings. Task failures and escaped tool panics return `isError: true`, and the server keeps
serving. A failed batch includes per-step errors and is marked `isError: true`.

Exports complete synchronously. A call's `progressToken` and cancellation notifications are
harmlessly ignored; the server does not provide background export progress or cancellation.

## A server limited by a grant (`mcp --client`)

`pdfcraft-cli mcp --client NAME [--state-dir DIR]` is the MCP server over the application service (BUILD-009). The client's
secret is read from the environment variable `BDF_CLIENT_SECRET`, never from an argument, which other processes can read. It
takes no `--root` or `--compact`: a client's folders are in its grant, and its tool list is the service's.

* **Without a valid grant** (no secret, a wrong secret, an unknown client, a revoked or expired grant, all answered the same)
  it lists one tool, `bdf_help`, which returns the version and the service's operations. It lists no files and returns no
  document content; every other tool is refused with `not_authorized`.
* **With a grant** the tools are `doc_open`, `doc_info`, `page_rotate`, `edit_undo`, `edit_redo`, `edit_history`, `doc_save`
  and `doc_close`, one per service operation, with the automation tools' names where there is one. What the grant allows is the
  policy's decision: a read-only grant lists the same tools and is refused at the call, with the reason. A revocation from
  another process stops the next call.
* A tool that changes or discards content takes `revision` (from the last reply); a stale one is a `conflict` that names both
  revisions and changes nothing. Page numbers are 1-based. Every document-tool call is recorded in the activity log (`pdfcraft-cli activity show`), including one refused for its arguments (unknown name, wrong type, missing value, a negative revision): the client, the grant, the operation, the outcome and the error's code, never what the client sent. `bdf_help`, the protocol's own messages, an unknown tool name and a call without a grant are not recorded. Several servers may run at once against one state folder; they take turns writing the log.
* Results are in `structuredContent`. A failure has `isError: true` and `{"error": {"code", "message", "retryable", "reason"?,
  "conflict"?, "committed"?}}`; the codes are the service's (`invalid_argument`, `not_authorized`, `not_found`, `conflict`, ...).

The server without `--client` is the older one described above: it is **not** limited by a grant and has the tool set of the
automation engine. It stays until the tools it offers are available through the service; see `docs/bdf-evidence/BUILD-009.md`.

## Resources and protocol versions

`pdfcraft://document` returns the session's document information (as `doc_inspect` without
arguments), including an empty `documents` array before any PDF is opened. `pdfcraft://commands`
returns the command catalog (as `command_list`). Existing per-document info, text and page-image
resources and their templates are preserved.

Unknown top-level argument keys are refused with JSON-RPC `-32602` (invalid params) instead of
being ignored, so a misspelled argument is reported rather than silently dropped. Clients that
sent extra keys before get an error now.
