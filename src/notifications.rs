//! Native desktop notifications.
//!
//! macOS posts through `UNUserNotificationCenter`: the older
//! `NSUserNotificationCenter` API behind `notify-rust` is deprecated and
//! silently drops reports on current macOS releases. Linux and Windows keep
//! `notify-rust`.

use std::io::Write;
use std::sync::mpsc::{sync_channel, SyncSender};
use std::sync::OnceLock;

const QUEUE_DEPTH: usize = 16;
const LOG_LIMIT: u64 = 64 * 1024;

/// Appends one line to `notifications.log` in the cache directory. Delivery
/// failures are otherwise invisible: a Finder launch sends stderr nowhere, so
/// a rejected notification looks like a missing one.
fn log(message: &str) {
    let path = crate::paths::get().cache_dir.join("notifications.log");
    let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    else {
        return;
    };
    if file
        .metadata()
        .map(|meta| meta.len() > LOG_LIMIT)
        .unwrap_or(false)
    {
        return;
    }
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0);
    let _ = writeln!(file, "{seconds} {message}");
}

/// Queue a native notification for delivery on the worker thread.
pub fn send_system_notification(title: &str, body: &str) {
    static WORKER: OnceLock<Result<SyncSender<(String, String)>, String>> = OnceLock::new();
    let worker = WORKER.get_or_init(|| {
        let (sender, receiver) = sync_channel::<(String, String)>(QUEUE_DEPTH);
        std::thread::Builder::new()
            .name("fastty-notifications".into())
            .spawn(move || {
                for (title, body) in receiver {
                    deliver(&title, &body);
                }
            })
            .map(|_| sender)
            .map_err(|error| error.to_string())
    });
    match worker {
        Ok(sender) => {
            if let Err(error) = sender.try_send((title.to_owned(), body.to_owned())) {
                log(&format!("queue rejected a report: {error}"));
            }
        }
        Err(error) => log(&format!("notification worker failed: {error}")),
    }
}

/// Ask macOS for notification authorization early so the system prompt shows
/// before the first agent report. Other platforms need no request.
#[cfg(target_os = "macos")]
pub fn request_permission() {
    macos::request_permission();
}

#[cfg(not(target_os = "macos"))]
pub fn request_permission() {}

#[cfg(target_os = "macos")]
fn deliver(title: &str, body: &str) {
    macos::deliver(title, body);
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn deliver(title: &str, body: &str) {
    let summary = if title.is_empty() { "Fastty" } else { title };
    #[cfg(target_os = "linux")]
    let body = body
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    if let Err(error) = notify_rust::Notification::new()
        .appname("Fastty")
        .summary(summary)
        .body(&body)
        .show()
    {
        eprintln!("Fastty notification delivery failed: {error}");
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
fn deliver(_title: &str, _body: &str) {}

#[cfg(target_os = "macos")]
mod macos {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::OnceLock;

    use block2::RcBlock;
    use objc2_06::msg_send;
    use objc2_06::runtime::{AnyObject, Bool};
    use objc2_foundation::{NSError, NSString};
    use objc2_user_notifications::{
        UNAuthorizationOptions, UNMutableNotificationContent, UNNotificationRequest,
        UNNotificationSound, UNUserNotificationCenter,
    };

    use super::log;

    /// True only when the process carries a bundle identifier. An ObjC
    /// exception here aborts the whole process, so this must run before any
    /// `UNUserNotificationCenter` call.
    fn unuser_notifications_available() -> bool {
        // SAFETY: plain message sends to NSBundle.mainBundle; both returns
        // are objects the caller never retains.
        unsafe {
            let bundle: *mut AnyObject = msg_send![objc2_06::class!(NSBundle), mainBundle];
            if bundle.is_null() {
                return false;
            }
            let identifier: *mut AnyObject = msg_send![bundle, bundleIdentifier];
            !identifier.is_null()
        }
    }

    pub fn request_permission() {
        static REQUESTED: OnceLock<()> = OnceLock::new();
        REQUESTED.get_or_init(|| {
            // `+[UNUserNotificationCenter currentNotificationCenter]` raises
            // when the process has no main bundle identifier — any bare
            // `fastty` CLI/GUI launch (crash report 5EC0FF51, 2026-10-25).
            // Skip instead of aborting; without a bundle there is no
            // notification identity to authorize anyway.
            if !unuser_notifications_available() {
                log("skipped: no bundle identity (bare binary launch)");
                return;
            }
            let center = UNUserNotificationCenter::currentNotificationCenter();
            let granted_block = RcBlock::new(|granted: Bool, _error: *mut NSError| {
                log(&format!("authorization granted={}", granted.as_bool()));
            });
            center.requestAuthorizationWithOptions_completionHandler(
                UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound,
                &granted_block,
            );
        });
    }

    pub fn deliver(title: &str, body: &str) {
        static NEXT_ID: AtomicU64 = AtomicU64::new(1);
        if !unuser_notifications_available() {
            log(&format!("dropped (no bundle identity): {title}"));
            return;
        }
        request_permission();
        let center = UNUserNotificationCenter::currentNotificationCenter();
        let content = UNMutableNotificationContent::new();
        content.setTitle(&NSString::from_str(title));
        content.setBody(&NSString::from_str(body));
        content.setSound(Some(&UNNotificationSound::defaultSound()));
        let identifier = NSString::from_str(&format!(
            "fastty-report-{}",
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        ));
        let request = UNNotificationRequest::requestWithIdentifier_content_trigger(
            &identifier,
            &content,
            None,
        );
        let report = title.to_owned();
        let add_block = RcBlock::new(move |error: *mut NSError| match unsafe { error.as_ref() } {
            Some(error) => log(&format!("add failed for '{report}': {error:?}")),
            None => log(&format!("posted '{report}'")),
        });
        center.addNotificationRequest_withCompletionHandler(&request, Some(&add_block));
    }
}

#[cfg(all(test, target_os = "macos"))]
mod unbundled_tests {
    /// Crash report 5EC0FF51 (2026-10-25): a bare `fastty` CLI launch has no
    /// main bundle identifier, and `+[UNUserNotificationCenter
    /// currentNotificationCenter]` raises there, aborting the process 1.3s
    /// after launch. Test binaries run in the same unbundled condition, so
    /// this test goes red (SIGABRT) exactly on that bug and stays green
    /// once the guards hold.
    #[test]
    fn request_permission_and_delivery_survive_without_a_bundle() {
        super::macos::request_permission();
        super::macos::deliver("Fastty", "unbundled probe");
    }
}
