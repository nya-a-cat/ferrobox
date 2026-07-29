# Performance

Ferrobox performance claims are based on retained GitHub-hosted KVM artifacts.
The benchmark records microseconds as integers and retains all cold-create,
hot-execution, and deletion samples so percentile calculations remain
auditable. Current hosted-KVM runs use five lifecycle samples and twenty hot
execution samples.

## Baseline

Run [30417577685](https://github.com/nya-a-cat/ferrobox/actions/runs/30417577685)
on `ubuntu-24.04` established the first baseline:

| Metric | Result |
| --- | ---: |
| create to guest ready | 2,384.304 ms |
| `/bin/true` P50 | 2.237 ms |
| `/bin/true` P95 | 7.544 ms |
| Python `print(42)` | 16.247 ms |
| delete | 3.228 ms |
| full measured lifecycle | 2,471.580 ms |

The run failed its initial two-second create ceiling and successfully retained
the JSON artifact. The failure is evidence that the gate is active.

Run [30417866400](https://github.com/nya-a-cat/ferrobox/actions/runs/30417866400)
measured the first channel-reuse revision:

| Metric | Baseline | Channel reuse |
| --- | ---: | ---: |
| create to guest ready | 2,384.304 ms | 2,540.730 ms |
| `/bin/true` P50 | 2.237 ms | 3.208 ms |
| `/bin/true` P95 | 7.544 ms | 3.868 ms |
| Python `print(42)` | 16.247 ms | 35.464 ms |
| delete | 3.228 ms | 1.582 ms |

The hot-command P95 improved by 48.7%. Single-sample create and Python results
show runner noise, so future cold-start evidence uses repeated samples before
claiming a latency improvement.

## Measurement boundary

Each `create_to_ready_us` sample starts before `FirecrackerRuntime::create` and
stops only after the guest health check and authenticated initialization
complete. It includes jail preparation, kernel/rootfs cloning, Jailer startup,
Firecracker configuration, Linux boot, vsock connection, guest health, and
initialization. The regression gate uses the retained create P95.

Hot execution includes host-side lookup, gRPC over virtio-vsock, guest process
spawn, process cgroup assignment, exit collection, and response delivery.

The benchmark excludes workflow checkout, compilation, rootfs construction,
and artifact upload.

## Targets

The optimization program tracks two different thresholds:

1. Regression ceilings derived from Ferrobox hosted-KVM history.
2. Competitive targets derived from a reproducible same-host harness.

The first competitive create-to-ready target is below 200 ms, matching the
public E2B product claim. It does not become a Ferrobox claim until the retained
benchmark artifact passes. Hot `/bin/true` targets are P50 below 1 ms and P95
below 2 ms after persistent vsock-channel reuse.

## Optimization order

1. Avoid immutable kernel copies and reuse established vsock/HTTP2 channels.
2. Capture a ready Firecracker memory/device snapshot.
3. Restore from the snapshot with a per-sandbox writable COW rootfs.
4. Add a bounded ready pool for allocation without restore latency.
5. Add concurrent-create throughput and host RSS measurements.
