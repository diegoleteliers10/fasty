use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::panic;

fn crashes_dir() -> PathBuf {
    crate::paths::get().state_dir.join("crashes")
}

pub fn install_hook() {
    let start_time = std::time::Instant::now();
    let default_hook = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current().name().unwrap_or("<unnamed>").to_string();

        let msg = if let Some(s) = info.payload().downcast_ref::<&str>() {
            s.to_string()
        } else if let Some(s) = info.payload().downcast_ref::<String>() {
            s.clone()
        } else {
            "unknown panic".to_string()
        };

        let location = info.location().map(|l| {
            format!("{}:{}:{}", l.file(), l.line(), l.column())
        }).unwrap_or_else(|| "unknown".to_string());

        let backtrace = std::backtrace::Backtrace::force_capture();

        let now = chrono_filename();
        let dir = crashes_dir();
        let _ = fs::create_dir_all(&dir);
        let path = dir.join(format!("crash_{now}.log"));

        let mut f = fs::File::create(&path).ok();
        if let Some(ref mut f) = f {
            let _ = writeln!(f, "Fastty Crash Report");
            let _ = writeln!(f, "==================");
            let _ = writeln!(f, "Version: {}", env!("CARGO_PKG_VERSION"));
            let _ = writeln!(f, "OS: {} {}", std::env::consts::OS, std::env::consts::ARCH);
            let _ = writeln!(f, "Date: {now}");
            let _ = writeln!(f, "Thread: {thread}");
            let _ = writeln!(f, "Message: \"{msg}\"");
            let _ = writeln!(f, "Location: {location}");
            let _ = writeln!(f);
            let _ = writeln!(f, "Backtrace:");
            let _ = writeln!(f, "{backtrace}");
        }

        eprintln!("fastty: crash saved to {}", path.display());

        let _ = default_hook(info);

        if start_time.elapsed().as_secs() > 5 {
            if let Ok(exe) = std::env::current_exe() {
                let _ = std::process::Command::new(exe).spawn();
            }
        } else {
            eprintln!("fastty: crash occurred within 5 seconds of startup, skipping auto-restart to prevent crash loop.");
        }
    }));
}

fn chrono_filename() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let secs_per_day = 86400u64;
    let days = (now / secs_per_day) as i64;
    let time_of_day = (now % secs_per_day) as u64;

    let (y, m, d) = days_to_ymd(days);
    let h = time_of_day / 3600;
    let min = (time_of_day % 3600) / 60;
    let s = time_of_day % 60;
    format!("{y:04}-{m:02}-{d:02}_{h:02}-{min:02}-{s:02}")
}

fn days_to_ymd(days: i64) -> (i64, u32, u32) {
    let z = days + 719468;
    let era = z / 146097;
    let doe = (z - era * 146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}
