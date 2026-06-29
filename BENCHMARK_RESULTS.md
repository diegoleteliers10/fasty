# Terminal Benchmark Report
Runs per test case: 3

| Terminal | Startup Latency (sh -c true) | Plain Text Throughput (100k lines) | ANSI/SGR Colors Throughput (50k lines) |
| --- | --- | --- | --- |
| **fastty (This Project)** | 1.053s ± 0.280s | 1.988s ± 0.079s | 1.942s ± 0.126s |
| **Ghostty** | 1.772s ± 0.305s | 2.405s ± 0.277s | 1.987s ± 0.253s |
| **Konsole (KDE Terminal)** | 0.684s ± 0.248s | 2.256s ± 0.493s | 1.202s ± 0.323s |