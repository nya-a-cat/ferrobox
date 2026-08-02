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

## Snapshots

Create and list use the source sandbox bearer token:

```http
POST /v1/sandboxes/{id}/snapshots
authorization: Bearer <sandbox-token>
content-type: application/json

{"name":"before-upgrade"}
```

The create response contains the snapshot metadata and a distinct snapshot
token. That token authorizes metadata, integrity verification, restore, clone,
and deletion after the source sandbox has been deleted:

- `GET /v1/snapshots/{snapshot_id}`
- `POST /v1/snapshots/{snapshot_id}/verify`
- `POST /v1/snapshots/{snapshot_id}/restore` with `{"timeout_seconds":300}`
- `POST /v1/snapshots/{snapshot_id}/clones` with
  `{"count":2,"timeout_seconds":300}`
- `DELETE /v1/snapshots/{snapshot_id}`

`GET /v1/sandboxes/{id}/snapshots?limit=50&cursor=<snapshot-id>` returns a
stable, creation-ordered page. Limits range from 1 through 100.

Rollback uses the source sandbox token and accepts a snapshot created from that
same sandbox:

```http
POST /v1/sandboxes/{id}/rollback/{snapshot_id}
authorization: Bearer <sandbox-token>
```

The call keeps the sandbox ID, bearer token, and expiry while atomically
replacing the underlying VM with a verified restore. See
[`snapshots.md`](snapshots.md) for consistency, artifact, and failure rules.
