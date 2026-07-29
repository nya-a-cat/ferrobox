#!/usr/bin/env python3
import argparse
import http.client
import json
import socket
import time


class UnixConnection(http.client.HTTPConnection):
    def __init__(self, socket_path: str) -> None:
        super().__init__("localhost", timeout=30)
        self.socket_path = socket_path

    def connect(self) -> None:
        self.sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.sock.settimeout(self.timeout)
        self.sock.connect(self.socket_path)


def percentile(sorted_samples: list[int], value: int) -> int:
    rank = (len(sorted_samples) * value + 99) // 100
    return sorted_samples[min(rank - 1, len(sorted_samples) - 1)]


def request(
    connection: UnixConnection,
    method: str,
    path: str,
    body: dict | None = None,
) -> tuple[int, bytes]:
    payload = None if body is None else json.dumps(body, separators=(",", ":"))
    headers = {} if payload is None else {"content-type": "application/json"}
    connection.request(method, path, body=payload, headers=headers)
    response = connection.getresponse()
    return response.status, response.read()


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--socket", default="/var/run/docker.sock")
    parser.add_argument("--image", required=True)
    parser.add_argument("--iterations", type=int, default=5)
    arguments = parser.parse_args()
    if arguments.iterations < 1 or arguments.iterations > 100:
        raise ValueError("iterations must be between 1 and 100")

    connection = UnixConnection(arguments.socket)
    samples: list[int] = []

    def create_container() -> str:
        status, body = request(
            connection,
            "POST",
            "/v1.44/containers/create",
            {
                "Image": arguments.image,
                "Cmd": ["sleep", "300"],
                "NetworkDisabled": True,
                "HostConfig": {
                    "Memory": 512 * 1024 * 1024,
                    "NanoCpus": 1_000_000_000,
                    "PidsLimit": 512,
                    "NetworkMode": "none",
                },
            },
        )
        if status != 201:
            raise RuntimeError(f"Docker create returned {status}: {body.decode()}")
        return json.loads(body)["Id"]

    def start_container(container_id: str) -> None:
        status, body = request(
            connection,
            "POST",
            f"/v1.44/containers/{container_id}/start",
        )
        if status != 204:
            raise RuntimeError(f"Docker start returned {status}: {body.decode()}")

    def delete_container(container_id: str) -> None:
        status, body = request(
            connection,
            "DELETE",
            f"/v1.44/containers/{container_id}?force=true",
        )
        if status != 204:
            raise RuntimeError(f"Docker delete returned {status}: {body.decode()}")

    for _ in range(arguments.iterations):
        started = time.perf_counter_ns()
        container_id = create_container()
        start_container(container_id)
        samples.append((time.perf_counter_ns() - started) // 1000)
        status, body = request(
            connection,
            "GET",
            f"/v1.44/containers/{container_id}/json",
        )
        if status != 200 or not json.loads(body)["State"]["Running"]:
            raise RuntimeError(f"Docker container did not reach running: {body.decode()}")
        delete_container(container_id)

    samples.sort()
    exec_container_id = create_container()
    start_container(exec_container_id)
    exec_samples: list[int] = []
    for _ in range(20):
        started = time.perf_counter_ns()
        status, body = request(
            connection,
            "POST",
            f"/v1.44/containers/{exec_container_id}/exec",
            {"Cmd": ["/bin/true"], "AttachStdout": False, "AttachStderr": False},
        )
        if status != 201:
            raise RuntimeError(f"Docker exec create returned {status}: {body.decode()}")
        exec_id = json.loads(body)["Id"]
        status, body = request(
            connection,
            "POST",
            f"/v1.44/exec/{exec_id}/start",
            {"Detach": True, "Tty": False},
        )
        if status != 200:
            raise RuntimeError(f"Docker exec start returned {status}: {body.decode()}")
        while True:
            status, body = request(
                connection,
                "GET",
                f"/v1.44/exec/{exec_id}/json",
            )
            if status != 200:
                raise RuntimeError(
                    f"Docker exec inspect returned {status}: {body.decode()}"
                )
            state = json.loads(body)
            if not state["Running"]:
                if state["ExitCode"] != 0:
                    raise RuntimeError(f"Docker exec failed: {state}")
                break
            time.sleep(0.0005)
        exec_samples.append((time.perf_counter_ns() - started) // 1000)
    delete_container(exec_container_id)
    exec_samples.sort()
    print(
        json.dumps(
            {
                "schema_version": 3,
                "docker_create_start_us": samples,
                "docker_create_start_p50_us": percentile(samples, 50),
                "docker_create_start_p95_us": percentile(samples, 95),
                "docker_exec_true_us": exec_samples,
                "docker_exec_true_p50_us": percentile(exec_samples, 50),
                "docker_exec_true_p95_us": percentile(exec_samples, 95),
            },
            indent=2,
        )
    )


if __name__ == "__main__":
    main()
