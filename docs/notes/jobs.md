# Agent jobs

A job is a run of the agent on the hub, started by an outside agent (Claude Code, Codex, a web chat) through the `jobs` tool. Use it to hand work to the hub's agent and pick up its result later. Jobs run only on the hub; a local MCP server answers that they need the hub.

## Starting a job

`jobs` with `action` `create` and a `prompt` starts a run and returns the job: its `id`, `status` (`queued` at first) and `tools`. The agent works from the prompt alone, so say everything it needs: which pages, what to change, what to report.

`create` also takes optional `page` (a page id) and `after` (a block id on that page). With `page`, the job puts a placeholder in the page, and its result replaces the placeholder when the job ends. `after` needs `page`; an unknown page gives "no such page". The job reports `page`.

The run gets your tool set, narrowed by the hub runner's tool set (Settings > AI > Agent, "Tools on the hub"). `tools` (`read`, `write` or `everything`) narrows it further; asking for more than your own set is refused. Callers with the Read set can't create jobs. The run never asks for approval: it simply has no tools outside its set. A signed-in device's jobs have no set of their own; they get the hub runner's set.

## Following a job

- `get` with `job` returns the job. Once it is `completed`, the job carries `result`, the agent's final text. A `failed` or `interrupted` job carries `error`.
- `wait` with `job` blocks until the job ends or `timeout_ms` passes (at most 60000), then returns it like `get`. Call it again to keep waiting.
- `cancel` with `job` stops a queued or running job; it ends `cancelled`.
- `list` returns your jobs, newest first (`limit`, default 20).

States: `queued`, `running`, then `completed`, `failed`, `cancelled` or `interrupted`. A job is `interrupted` when the hub restarted while it ran; it is not started again, so create a new one.

## Who sees a job

Only the caller that created a job sees it, waits for it or cancels it. Another token gets "no such job". Any signed-in device may list and cancel device-created jobs; agent tokens never see them. Each job is also a row of the "AI tasks" database in the vault, so the user's devices see every job and its result.

## Limits

- One running job per creator; further jobs wait in the queue.
- At most 4 queued jobs on the hub; `create` is refused while the queue is full.
- A run stops after 1 hour and the job fails.
- Ended jobs are kept 30 days, then move to the trash.
