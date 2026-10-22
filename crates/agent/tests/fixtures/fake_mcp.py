"""A tiny stdio MCP server for the agent tests: one tool, `echo`."""
import json
import sys

ECHO = {
    "name": "echo",
    "description": "Returns its text.",
    "inputSchema": {
        "type": "object",
        "properties": {"text": {"type": "string"}},
        "required": ["text"],
    },
}


def reply(id, result):
    sys.stdout.write(json.dumps({"jsonrpc": "2.0", "id": id, "result": result}) + "\n")
    sys.stdout.flush()


for line in sys.stdin:
    if not line.strip():
        continue
    msg = json.loads(line)
    method, id = msg.get("method"), msg.get("id")
    if id is None:
        continue
    if method == "initialize":
        reply(id, {
            "protocolVersion": "2025-06-18",
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "fake", "version": "0"},
        })
    elif method == "tools/list":
        reply(id, {"tools": [ECHO]})
    elif method == "tools/call":
        text = msg["params"]["arguments"].get("text", "")
        reply(id, {"content": [{"type": "text", "text": "echo: " + text}]})
    else:
        sys.stdout.write(json.dumps({
            "jsonrpc": "2.0", "id": id,
            "error": {"code": -32601, "message": "unknown method"},
        }) + "\n")
        sys.stdout.flush()
