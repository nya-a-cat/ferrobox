#!/usr/bin/env python3
import argparse
import concurrent.futures
import http.client
import json
import time
from urllib.parse import urlsplit


def percentile(sorted_samples: list[int], value: int) -> int:
    rank = (len(sorted_samples) * value + 99) // 100
    return sorted_samples[min(rank - 1, len(sorted_samples) - 1)]


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--url", required=True)
    parser.add_argument("--iterations", type=int, default=5)
    parser.add_argument("--concurrency", type=int, default=5)
    arguments = parser.parse_args()
    if (
        arguments.iterations < 1
        or arguments.iterations > 100
        or arguments.concurrency < 1
        or arguments.concurrency > 32
    ):
        raise ValueError("iterations/concurrency are outside supported limits")

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

    def create_and_delete(client: http.client.HTTPConnection) -> int:
        started = time.perf_counter_ns()
        client.request(
            "POST",
            "/v1/sandboxes",
            body=request_body,
            headers={"content-type": "application/json"},
        )
        response = client.getresponse()
        body = response.read()
        elapsed = (time.perf_counter_ns() - started) // 1000
        if response.status != 201:
            raise RuntimeError(f"create returned {response.status}: {body.decode()}")
        created = json.loads(body)
        client.request(
            "DELETE",
            f"/v1/sandboxes/{created['sandbox_id']}",
            headers={"authorization": f"Bearer {created['token']}"},
        )
        deleted = client.getresponse()
        deleted_body = deleted.read()
        if deleted.status != 204:
            raise RuntimeError(
                f"delete returned {deleted.status}: {deleted_body.decode()}"
            )
        return elapsed

    samples = [create_and_delete(connection) for _ in range(arguments.iterations)]
    concurrent_started = time.perf_counter_ns()
    with concurrent.futures.ThreadPoolExecutor(
        max_workers=arguments.concurrency
    ) as executor:
        concurrent_samples = list(
            executor.map(
                lambda _: create_and_delete(
                    http.client.HTTPConnection(
                        endpoint.hostname, endpoint.port, timeout=10
                    )
                ),
                range(arguments.concurrency),
            )
        )
    concurrent_wall_us = (time.perf_counter_ns() - concurrent_started) // 1000

    samples.sort()
    concurrent_samples.sort()
    print(
        json.dumps(
            {
                "schema_version": 3,
                "http_create_us": samples,
                "http_create_p50_us": percentile(samples, 50),
                "http_create_p95_us": percentile(samples, 95),
                "concurrent_create_us": concurrent_samples,
                "concurrent_create_p50_us": percentile(concurrent_samples, 50),
                "concurrent_create_p95_us": percentile(concurrent_samples, 95),
                "concurrent_create_wall_us": concurrent_wall_us,
            },
            indent=2,
        )
    )


if __name__ == "__main__":
    main()
