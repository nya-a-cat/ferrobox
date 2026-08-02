def nearest_rank($values; $percent):
  ($values | sort) as $sorted
  | (((($sorted | length) * $percent) + 99) / 100 | floor) as $rank
  | $sorted[$rank - 1];

.[0] as $firecracker_a
| .[1] as $cloud_hypervisor_a
| .[2] as $cloud_hypervisor_b
| .[3] as $firecracker_b
| ([$firecracker_a.exec_true_cloned_client_us[], $firecracker_b.exec_true_cloned_client_us[]] | sort) as $firecracker_true
| ([$cloud_hypervisor_a.exec_true_cloned_client_us[], $cloud_hypervisor_b.exec_true_cloned_client_us[]] | sort) as $cloud_hypervisor_true
| ([$firecracker_a.exec_python_us[], $firecracker_b.exec_python_us[]] | sort) as $firecracker_python
| ([$cloud_hypervisor_a.exec_python_us[], $cloud_hypervisor_b.exec_python_us[]] | sort) as $cloud_hypervisor_python
| ([$firecracker_a.ready_us[], $firecracker_b.ready_us[]] | sort) as $firecracker_ready
| ([$cloud_hypervisor_a.ready_us[], $cloud_hypervisor_b.ready_us[]] | sort) as $cloud_hypervisor_ready
| {
    schema_version: 1,
    sequence: [
      "firecracker-cpu-capped-a",
      "cloud-hypervisor-a",
      "cloud-hypervisor-b",
      "firecracker-cpu-capped-b"
    ],
    source_schema_versions: map(.schema_version),
    firecracker: {
      runtime: $firecracker_a.runtime,
      host_cpu_max: $firecracker_a.host_cpu_max,
      ready_us: $firecracker_ready,
      ready_p50_us: nearest_rank($firecracker_ready; 50),
      ready_p95_us: nearest_rank($firecracker_ready; 95),
      exec_true_us: $firecracker_true,
      exec_true_p50_us: nearest_rank($firecracker_true; 50),
      exec_true_p95_us: nearest_rank($firecracker_true; 95),
      exec_python_us: $firecracker_python,
      exec_python_p50_us: nearest_rank($firecracker_python; 50),
      exec_python_p95_us: nearest_rank($firecracker_python; 95)
    },
    cloud_hypervisor: {
      runtime: $cloud_hypervisor_a.runtime,
      ready_us: $cloud_hypervisor_ready,
      ready_p50_us: nearest_rank($cloud_hypervisor_ready; 50),
      ready_p95_us: nearest_rank($cloud_hypervisor_ready; 95),
      exec_true_us: $cloud_hypervisor_true,
      exec_true_p50_us: nearest_rank($cloud_hypervisor_true; 50),
      exec_true_p95_us: nearest_rank($cloud_hypervisor_true; 95),
      exec_python_us: $cloud_hypervisor_python,
      exec_python_p50_us: nearest_rank($cloud_hypervisor_python; 50),
      exec_python_p95_us: nearest_rank($cloud_hypervisor_python; 95)
    }
  }
