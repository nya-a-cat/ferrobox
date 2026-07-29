#!/usr/bin/env python3
import argparse
import http.client
import json
import time
from urllib.parse import urlsplit


def percentile(sorted_samples: list[int], value: int) -> int:
    return sorted_samples[(len(sorted_samples) - 1) * value // 100]


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--url", required=True)
    parser.add_argument("--iterations", type=int, default=5)
    arguments = parser.parse_args()
    if arguments.iterations < 1 or arguments.iterations > 100:
        raise ValueError("iterations must be between 1 and 100")

    endpoint = urlsplit(arguments.url)
    connection = http.client.HTTPConnection(endpoint.hostname, endpoint.port, timeout=10)
    request_body = json.dumps(
        {
            "template": "python",
            "cpu_count": 1,
            "memory_mb": 512,
            "timeout_seconds": 120,
            "network": {"internet_access": False},
        },
        separators=(",", ":"),
    )
    samples: list[int] = []
    for _ in range(arguments.iterations):
        started = time.perf_counter_ns()
        connection.request(
            "POST",
            "/v1/sandboxes",
            body=request_body,
            headers={"content-type": "application/json"},
        )
        response = connection.getresponse()
        body = response.read()
        samples.append((time.perf_counter_ns() - started) // 1000)
        if response.status != 201:
            raise RuntimeError(f"create returned {response.status}: {body.decode()}")
        created = json.loads(body)
        connection.request(
            "DELETE",
            f"/v1/sandboxes/{created['sandbox_id']}",
            headers={"authorization": f"Bearer {created['token']}"},
        )
        deleted = connection.getresponse()
        deleted_body = deleted.read()
        if deleted.status != 204:
            raise RuntimeError(
                f"delete returned {deleted.status}: {deleted_body.decode()}"
            )

    samples.sort()
    print(
        json.dumps(
            {
                "schema_version": 1,
                "http_create_us": samples,
                "http_create_p50_us": percentile(samples, 50),
                "http_create_p95_us": percentile(samples, 95),
            },
            indent=2,
        )
    )


if __name__ == "__main__":
    main()
