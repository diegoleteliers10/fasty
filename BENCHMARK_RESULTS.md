# Terminal Benchmark Report
Runs per test case: 3

| Terminal | Startup Latency (sh -c true) | Plain Text Throughput (100k lines) | ANSI/SGR Colors Throughput (50k lines) |
| --- | --- | --- | --- |
| **fasty (This Project)** | 9.115s ± 0.410s | 12.289s ± 0.408s | 10.566s ± 0.386s |
| **Ghostty** | 3.111s ± 0.305s | 4.802s ± 0.401s | 3.780s ± 0.282s |
| **Konsole (KDE Terminal)** | 1.019s ± 0.393s | 5.300s ± 0.153s | 2.174s ± 0.313s |