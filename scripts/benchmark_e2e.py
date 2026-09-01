#!/usr/bin/env python3
import subprocess
import time
import os
import sys
import tempfile
import statistics
import shutil
import platform

# Number of runs per benchmark to ensure statistical significance
RUNS = 3

# Number of untimed warm-up launches per test case, discarded before the
# timed runs. GUI terminals pay a one-off cold-start cost (shader/font
# cache, window server handshake, etc.) on the very first launch after
# boot; without a warm-up that outlier skews the average heavily.
WARMUP_RUNS = 1

IS_MACOS = platform.system() == "Darwin"

def macos_app_binary(app_name, binary_name):
    """Path to a terminal's CLI-invokable binary embedded inside its .app bundle.

    GUI terminals installed as macOS .app bundles (Ghostty.app, Alacritty.app,
    kitty.app, WezTerm.app, ...) generally aren't on PATH, but the app bundle
    embeds a real Mach-O binary at Contents/MacOS/<name> that can be invoked
    directly like any CLI program.
    """
    path = f"/Applications/{app_name}.app/Contents/MacOS/{binary_name}"
    return path if os.path.exists(path) else None

# Terminal definitions and their command line syntax to run a command and auto-close.
# `macos_app`/`macos_binary` are used only on macOS to locate the binary inside the
# app bundle when it isn't on PATH; `binary` is the fallback (and the only thing used
# on Linux/Windows).
TERMINALS = {
    "fastty (This Project)": {
        "binary": "./target/release/fastty",
        "cmd_args": lambda cmd: ["-e"] + cmd,
    },
    "Ghostty": {
        "binary": "ghostty",
        "macos_app": "Ghostty",
        "macos_binary": "ghostty",
        # Ghostty defaults to wait-after-command=false (the surface closes when
        # the executed command exits) but quit-after-last-window-closed=false
        # (the whole PROCESS keeps running with zero windows) -- without this
        # override, run_trial's "wait for process exit" never returns.
        # Known issue: on this machine (macOS 26/27 beta, Ghostty 1.3.1),
        # even with this override the window/process still hangs open most
        # of the time when invoked this way outside a normal Finder/Dock
        # launch -- confirmed via `sample` to be idling in NSApplication's
        # event loop rather than doing any work. Ghostty's own --help says
        # direct CLI launch is unsupported on macOS ("use open -na
        # Ghostty.app instead"), and `open -na` was tried too with the same
        # result, so this looks like a Ghostty-side timing bug on this OS
        # build rather than something fixable from here.
        "cmd_args": lambda cmd: ["--quit-after-last-window-closed=true", "-e"] + cmd,
    },
    "Alacritty": {
        "binary": "alacritty",
        "macos_app": "Alacritty",
        "macos_binary": "alacritty",
        "cmd_args": lambda cmd: ["-e"] + cmd,
    },
    "Kitty": {
        "binary": "kitty",
        "macos_app": "kitty",
        "macos_binary": "kitty",
        "cmd_args": lambda cmd: cmd,
    },
    "WezTerm": {
        "binary": "wezterm",
        "macos_app": "WezTerm",
        "macos_binary": "wezterm",
        "cmd_args": lambda cmd: ["start", "--"] + cmd,
    },
    "GNOME Terminal": {
        "binary": "gnome-terminal",
        "cmd_args": lambda cmd: ["--wait", "--"] + cmd,
    },
    "Konsole (KDE Terminal)": {
        "binary": "konsole",
        "cmd_args": lambda cmd: ["-e"] + cmd,
    },
    "xterm": {
        "binary": "xterm",
        "cmd_args": lambda cmd: ["-e"] + cmd,
    }
}

def resolve_binary(config):
    """Resolve the actual invokable path for a terminal's binary, or None if absent."""
    binary = config["binary"]
    if binary.startswith("./") or binary.startswith("/"):
        return os.path.abspath(binary) if os.path.exists(binary) else None
    if IS_MACOS and "macos_app" in config:
        found = macos_app_binary(config["macos_app"], config["macos_binary"])
        if found:
            return found
    on_path = shutil.which(binary)
    if on_path:
        return on_path
    return None

def check_binary(name, config):
    """Check if the terminal binary exists on the system."""
    return resolve_binary(config) is not None

def generate_payloads():
    """Generate temporary test files for benchmarking text rendering/processing."""
    print("Generating benchmark payloads...")
    temp_dir = tempfile.gettempdir()
    
    # 1. Plain text payload (100,000 lines)
    plain_path = os.path.join(temp_dir, "fastty_bench_plain.txt")
    with open(plain_path, "w") as f:
        for i in range(100000):
            f.write(f"Line {i:06d}: This is a plain text line to measure standard terminal writing throughput.\n")
            
    # 2. ANSI/SGR color payload (50,000 lines of highly styled text)
    ansi_path = os.path.join(temp_dir, "fastty_bench_ansi.txt")
    with open(ansi_path, "w") as f:
        for i in range(50000):
            # Rainbow/varying colors using SGR foreground & background formatting
            fg = 30 + (i % 8)
            bg = 40 + ((i + 3) % 8)
            style = i % 3
            f.write(f"\x1b[{style};{fg};{bg}mLine {i:06d}: \x1b[0m\x1b[1;32mGreen Text\x1b[0m and \x1b[31;4mUnderlined Red\x1b[0m\n")
            
    return plain_path, ansi_path

def run_trial(term_name, term_config, cmd):
    """Run a single benchmark trial and return the elapsed time."""
    binary_path = resolve_binary(term_config)
    if binary_path is None:
        print(f"  [Error] Could not resolve binary for {term_name}")
        return None

    full_cmd = [binary_path] + term_config["cmd_args"](cmd)
    
    start_time = time.perf_counter()
    try:
        # Run process, wait for it to finish and exit
        result = subprocess.run(
            full_cmd,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            timeout=30 # 30 seconds safety timeout
        )
        elapsed = time.perf_counter() - start_time
        if result.returncode != 0:
            # Some terminals exit with a non-zero code when cmd exits, ignore minor issues
            pass
        return elapsed
    except subprocess.TimeoutExpired:
        print(f"  [Timeout] {term_name} exceeded 30 seconds.")
        return None
    except Exception as e:
        print(f"  [Error] Failed running {term_name}: {e}")
        return None

def benchmark_terminal(term_name, term_config, test_cases):
    """Run all test cases for a specific terminal."""
    results = {}
    for case_name, cmd in test_cases.items():
        print(f"  Running '{case_name}'...", end="", flush=True)

        # Untimed warm-up launches, discarded, to absorb one-off cold-start
        # costs (shader/font cache, window server handshake) before timing.
        for _ in range(WARMUP_RUNS):
            run_trial(term_name, term_config, cmd)
            time.sleep(0.2)

        times = []
        for _ in range(RUNS):
            # Run the command and capture elapsed time
            elapsed = run_trial(term_name, term_config, cmd)
            if elapsed is not None:
                times.append(elapsed)
            time.sleep(0.2) # cooldown between runs
            
        if times:
            avg = statistics.mean(times)
            std_dev = statistics.stdev(times) if len(times) > 1 else 0.0
            results[case_name] = {"avg": avg, "std": std_dev}
            print(f" Done ({avg:.3f}s ± {std_dev:.3f}s)")
        else:
            results[case_name] = None
            print(" Failed")
    return results

def compile_fastty():
    """Ensure fastty is compiled in release mode before benchmarking."""
    print("Compiling fastty in --release mode...")
    try:
        subprocess.run(
            ["cargo", "build", "--release"],
            check=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT
        )
        print("Fastty compiled successfully.")
        return True
    except subprocess.CalledProcessError as e:
        print("ERROR: Failed to compile fastty in release mode.", file=sys.stderr)
        print(e.output.decode(), file=sys.stderr)
        return False

def main():
    print("=" * 60)
    print("       FASTTY TERMINAL EMULATOR E2E BENCHMARK RUNNER       ")
    print("=" * 60)
    
    # 1. Compile Fastty
    if not compile_fastty():
        sys.exit(1)
        
    # 2. Check for installed terminals
    active_terminals = {}
    for name, config in TERMINALS.items():
        if check_binary(name, config):
            active_terminals[name] = config
        else:
            print(f"ℹ️ {name} not found. Skipping.")
            
    if not active_terminals:
        print("❌ No terminals found to benchmark! (Did release compile of fastty fail?)")
        sys.exit(1)
        
    print(f"Detected {len(active_terminals)} terminals for evaluation.")
    
    # 3. Generate payload files
    plain_txt, ansi_txt = generate_payloads()
    
    # 4. Define test cases
    # - Startup: quickly launch and exit to measure startup latency
    # - Plain Text: cat a large 100k line text file
    # - ANSI/SGR Colors: cat a 50k line colorful text file
    test_cases = {
        "Startup Latency (sh -c true)": ["sh", "-c", "true"],
        "Plain Text Throughput (100k lines)": ["cat", plain_txt],
        "ANSI/SGR Colors Throughput (50k lines)": ["cat", ansi_txt],
    }
    
    # 5. Execute Benchmarks
    all_results = {}
    for name, config in active_terminals.items():
        print(f"\nBenchmarking {name}:")
        all_results[name] = benchmark_terminal(name, config, test_cases)
        
    # 6. Generate Markdown Report
    print("\n" + "=" * 60)
    print("                     BENCHMARK RESULTS                      ")
    print("=" * 60)
    
    report = []
    report.append("# Terminal Benchmark Report")
    report.append(f"Runs per test case: {RUNS}")
    report.append("")
    
    # Build Table Header
    headers = ["Terminal"]
    for case_name in test_cases.keys():
        headers.append(case_name)
    
    report.append("| " + " | ".join(headers) + " |")
    report.append("| " + " | ".join(["---"] * len(headers)) + " |")
    
    # Build Rows
    for term_name in active_terminals.keys():
        row = [f"**{term_name}**"]
        term_res = all_results[term_name]
        for case_name in test_cases.keys():
            res = term_res.get(case_name)
            if res:
                row.append(f"{res['avg']:.3f}s ± {res['std']:.3f}s")
            else:
                row.append("N/A")
        report.append("| " + " | ".join(row) + " |")
        
    report_md = "\n".join(report)
    print(report_md)
    print("\n" + "=" * 60)
    
    # Save to file
    report_file = "BENCHMARK_RESULTS.md"
    with open(report_file, "w") as f:
        f.write(report_md)
    print(f"Results written to {report_file}")
    
    # Cleanup temporary payloads
    try:
        os.remove(plain_txt)
        os.remove(ansi_txt)
    except:
        pass

if __name__ == "__main__":
    main()
