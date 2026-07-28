# HTTP API

The API listens on loopback by default. Each create response returns a
short-lived bearer token that authorizes only the created sandbox.

## Create

```http
POST /v1/sandboxes
content-type: application/json

{
  "template": "python",
  "cpu_count": 1,
  "memory_mb": 512,
  "timeout_seconds": 300,
  "network": {
    "internet_access": false
  }
}
```

The response contains `sandbox_id`, `node_id`, `state`, `token`, and token
expiry. The token is shown once and stored only as a SHA-256 digest.

## Execute

```http
POST /v1/sandboxes/{id}/commands
authorization: Bearer <token>
content-type: application/json

{
  "argv": ["python3", "-c", "print(42)"],
  "cwd": "/home/sandbox",
  "environment": {},
  "timeout_seconds": 30,
  "max_output_bytes": 1048576
}
```

Arguments are executed directly. Shell behavior requires an explicit shell
entry such as `["/bin/bash", "-lc", "..."]`.

The response contains the process ID, structured termination reason, UTF-8
display strings, lossless base64 stdout/stderr, and truncation flags.

## Files

Write:

```http
PUT /v1/sandboxes/{id}/files
authorization: Bearer <token>
content-type: application/json

{
  "path": "/home/sandbox/input.txt",
  "content_base64": "aGVsbG8K",
  "overwrite": false
}
```

Read:

```http
GET /v1/sandboxes/{id}/files?path=/home/sandbox/input.txt
authorization: Bearer <token>
```

Directory listing is available at
`GET /v1/sandboxes/{id}/directories?path=/home/sandbox`.

## Lifecycle

- `GET /v1/sandboxes/{id}`
- `POST /v1/sandboxes/{id}/pause`
- `POST /v1/sandboxes/{id}/resume`
- `DELETE /v1/sandboxes/{id}`

Expired sandboxes are deleted by the TTL reaper. A deleted ID and token no
longer authorize commands or files.

