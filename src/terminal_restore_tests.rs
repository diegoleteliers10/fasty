use super::*;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::{Config, Term};
use alacritty_terminal::vte::ansi::{Color, NamedColor};

fn source(bytes: &[u8], cols: usize, rows: usize) -> TerminalState {
    let terminal = TerminalState::new_headless("/bin/cat", &[], None, 20, 5).unwrap();
    terminal.terminate_process();
    let mut term = terminal.term.lock();
    term.resize(TermSize::new(cols, rows));
    let mut ingress = terminal.ingress.lock();
    ingress.processor.advance(&mut *term, bytes);
    for &byte in bytes {
        ingress.side.feed(byte);
        ingress.side.csi_watcher.feed(byte);
    }
    drop(ingress);
    drop(term);
    terminal
}

fn restored(source: &TerminalState) -> Term<EventListenerProxy> {
    let data = source
        .snapshot_state_for_save()
        .expect("capture native state");
    let snapshot = crate::server::native_snapshot::decode(&data).expect("decode native state");
    let proxy = EventListenerProxy::from_arc(Arc::new(ParkingMutex::new(
        Box::new(std::io::sink()) as Box<dyn Write + Send>,
    )));
    let mut term = Term::new(
        Config::default(),
        &TermSize::new(snapshot.main.columns, snapshot.main.screen_lines),
        proxy,
    );
    term.restore_snapshot(snapshot)
        .expect("restore native state");
    term
}

#[test]
fn palette_identity() {
    let source = source(b"\x1b[31mX", 4, 3);
    let target = restored(&source);
    assert_eq!(
        target.grid()[Line(0)][Column(0)].fg,
        Color::Named(NamedColor::Red)
    );
}

#[test]
fn active_style_continuation() {
    let source = source(b"\x1b[31mX", 4, 3);
    let mut target = restored(&source);
    let mut parser: Processor<StdSyncHandler> = Processor::new();
    parser.advance(&mut *source.term.lock(), b"Y");
    parser.advance(&mut target, b"Y");
    assert_eq!(
        target.grid()[Line(0)][Column(1)].fg,
        source.term.lock().grid()[Line(0)][Column(1)].fg
    );
}

#[test]
fn pending_wrap_continuation() {
    let source = source(b"ABCD", 4, 3);
    let mut target = restored(&source);
    let mut parser: Processor<StdSyncHandler> = Processor::new();
    parser.advance(&mut *source.term.lock(), b"E");
    parser.advance(&mut target, b"E");
    assert_eq!(
        target.grid()[Line(1)][Column(0)].c,
        source.term.lock().grid()[Line(1)][Column(0)].c
    );
}

#[test]
fn resize_reflow() {
    let source = source(b"ABCDEF", 4, 3);
    let mut target = restored(&source);
    source.term.lock().resize(TermSize::new(8, 3));
    target.resize(TermSize::new(8, 3));
    assert_eq!(
        target.grid()[Line(0)][Column(4)].c,
        source.term.lock().grid()[Line(0)][Column(4)].c
    );
}

#[test]
fn combining_character() {
    let source = source("e\u{301}".as_bytes(), 4, 3);
    let target = restored(&source);
    let source_term = source.term.lock();
    assert_eq!(
        target.grid()[Line(0)][Column(0)].zerowidth(),
        source_term.grid()[Line(0)][Column(0)].zerowidth()
    );
}

#[test]
fn shell_after_tui_exit() {
    let source = source(b"shell\x1b[?1049hTUI", 10, 3);
    let mut target = restored(&source);
    let mut parser: Processor<StdSyncHandler> = Processor::new();
    parser.advance(&mut *source.term.lock(), b"\x1b[?1049l");
    parser.advance(&mut target, b"\x1b[?1049l");
    assert_eq!(
        target.grid()[Line(0)][Column(0)].c,
        source.term.lock().grid()[Line(0)][Column(0)].c
    );
}

#[test]
fn compressed_saved_state_preserves_live_continuation() {
    let original = source(b"\x1b[31mABCD", 4, 3);
    let raw = original.snapshot_state_for_save().unwrap();
    let compressed = crate::server::binary_snapshot::compress_snapshot(&raw).unwrap();
    let target = source(b"old", 10, 5);
    assert!(target.restore_binary_snapshot(&compressed));
    let mut parser: Processor<StdSyncHandler> = Processor::new();
    parser.advance(&mut *target.term.lock(), b"E");
    let term = target.term.lock();
    assert_eq!(term.grid()[Line(1)][Column(0)].c, 'E');
    assert_eq!(
        term.grid()[Line(1)][Column(0)].fg,
        Color::Named(NamedColor::Red)
    );
}

#[test]
fn malformed_state_does_not_resize_or_change_the_terminal() {
    let target = source(b"safe", 10, 5);
    let before = target.capture_terminal_snapshot();
    let mut raw = target.snapshot_state_for_save().unwrap();
    raw[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(!target.restore_binary_snapshot(&raw));
    assert_eq!(target.capture_terminal_snapshot(), before);
}

#[test]
fn historical_shell_restore_resets_input_style_and_keeps_printed_colors() {
    let original = source(b"\x1b[31;44mX", 10, 5);
    let raw = original.snapshot_state_for_save().unwrap();
    let target = source(b"old", 10, 5);
    assert!(target.restore_saved_snapshot(&raw));
    let term = target.term.lock();
    assert_eq!(
        term.grid()[Line(0)][Column(0)].bg,
        Color::Named(NamedColor::Blue)
    );
    assert_eq!(
        term.grid().cursor.template.bg,
        Color::Named(NamedColor::Background)
    );
    assert_eq!(
        term.grid().cursor.template.fg,
        Color::Named(NamedColor::Foreground)
    );
}

#[test]
fn native_restore_keeps_scroll_region_saved_cursor_and_tab_stops() {
    let original = source(b"\x1b[3g\x1b[2;3H\x1bH\x1b7\x1b[2;4r", 10, 5);
    let mut target = restored(&original);
    let before = original.capture_terminal_snapshot();
    assert_eq!(target.snapshot(100), before);
    let mut parser: Processor<StdSyncHandler> = Processor::new();
    parser.advance(&mut target, b"\x1b8");
    assert_eq!(target.grid().cursor.point.line, Line(1));
    assert_eq!(target.grid().cursor.point.column, Column(2));
}

#[test]
fn native_parser_continues_at_each_byte_split() {
    use crate::server::native_snapshot::{self, Continuation, Encoding, NativeState};
    use alacritty_terminal::event::VoidListener;
    let sequences: &[&[u8]] = &[
        "Aé€🦀e\u{301}Z".as_bytes(),
        b"A\x1b[38:2::123:45:67;48;5;17mB\x1b[0mC",
        b"A\x1b[2;\n3HB",
        b"A\x1b]2;window title\x07B",
        b"A\x1b]2;window title\x1b\\B",
        b"A\x1b]8;id=link;https://example.com\x1b\\B\x1b]8;;\x1b\\C",
        b"A\x1bP1;2$qignored\x1b\\B",
        b"A\x1b_secret control string\x1b\\B",
        b"A\x1b_Ga=q,i=7;chunk\x1b\\B",
        b"A\x1b^private message\x1b\\B",
        b"A\x1bXstart of string\x1b\\B",
        b"A\x1b[?2026hB\x1b[31mC\x1b[?2026lD",
        b"A\x1b[3bB",
        b"A\x1b[12\x18B\x1b[34mC",
    ];
    for &sequence in sequences {
        for split in 0..=sequence.len() {
            for encoding in [Encoding::Raw, Encoding::Deflate] {
                let size = TermSize::new(20, 5);
                let mut source = Term::new(Config::default(), &size, VoidListener);
                let mut processor: Processor<StdSyncHandler> = Processor::new();
                processor.advance(&mut source, &sequence[..split]);
                let mut scanner = SideCheckpoint::default();
                for &byte in &sequence[..split] {
                    scanner.feed(byte);
                    scanner.csi_watcher.feed(byte);
                }
                let state = NativeState {
                    terminal: source.snapshot(100),
                    continuation: Some(Continuation {
                        parser: processor.snapshot().expect("save parser"),
                        side: scanner.clone(),
                    }),
                };
                let wire = native_snapshot::encode_state(&state, encoding).unwrap();
                let saved = crate::server::binary_snapshot::compress_snapshot(&wire).unwrap();
                let restored = native_snapshot::decode_state(&saved).unwrap();
                let mut target = Term::new(Config::default(), &size, VoidListener);
                target.restore_snapshot(restored.terminal).unwrap();
                let continuation = restored.continuation.unwrap();
                let mut resumed: Processor<StdSyncHandler> =
                    Processor::from_snapshot(continuation.parser).unwrap();
                let mut resumed_scanner = continuation.side;
                for &byte in &sequence[split..] {
                    assert_eq!(
                        resumed_scanner.feed(byte),
                        scanner.feed(byte),
                        "frame split {split}"
                    );
                    assert_eq!(
                        resumed_scanner.csi_watcher.feed(byte),
                        scanner.csi_watcher.feed(byte),
                        "CSI split {split}"
                    );
                }
                assert_eq!(
                    serde_json::to_value(&resumed_scanner).unwrap(),
                    serde_json::to_value(&scanner).unwrap()
                );
                processor.advance(&mut source, &sequence[split..]);
                resumed.advance(&mut target, &sequence[split..]);
                assert_eq!(
                    target.snapshot(100),
                    source.snapshot(100),
                    "split {split}, {sequence:?}"
                );
                assert_eq!(
                    serde_json::to_value(resumed.snapshot().unwrap()).unwrap(),
                    serde_json::to_value(processor.snapshot().unwrap()).unwrap(),
                    "parser split {split}, {sequence:?}",
                );
            }
        }
    }
}

#[test]
fn parser_bounds_are_checked_before_terminal_restore() {
    let original = source(b"safe", 10, 5);
    let before = original.capture_terminal_snapshot();
    let mut state = serde_json::to_value(original.capture_native_state().unwrap()).unwrap();
    state["continuation"]["parser"]["parser"]["partial_utf8_len"] = serde_json::json!(5);
    let payload = serde_json::to_vec(&state).unwrap();
    let mut wire = b"FST2".to_vec();
    wire.extend_from_slice(&1u16.to_le_bytes());
    wire.extend_from_slice(&0u16.to_le_bytes());
    wire.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    wire.extend_from_slice(&payload);
    assert!(!original.restore_binary_snapshot(&wire));
    assert_eq!(original.capture_terminal_snapshot(), before);
}

#[test]
fn historical_restore_clears_partial_parser_input() {
    let original = source(b"A\x1b]2;unfinished", 10, 5);
    let wire = original.snapshot_state_for_save().unwrap();
    let target = source(b"old", 10, 5);
    assert!(target.restore_saved_snapshot(&wire));
    let mut term = target.term.lock();
    target.ingress.lock().processor.advance(&mut *term, b"B");
    assert_eq!(term.grid()[Line(0)][Column(1)].c, 'B');
}

#[test]
fn malformed_parameter_groups_are_rejected_before_restore() {
    let original = source(b"safe", 10, 5);
    let before = original.capture_terminal_snapshot();
    let mut state = serde_json::to_value(original.capture_native_state().unwrap()).unwrap();
    let parser = &mut state["continuation"]["parser"]["parser"];
    parser["state"] = serde_json::json!("CsiParam");
    parser["ignoring"] = serde_json::json!(false);
    parser["params"]["len"] = serde_json::json!(2);
    parser["params"]["current_subparams"] = serde_json::json!(1);
    parser["params"]["subparams"] = serde_json::json!([
        2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0,
    ]);
    let payload = serde_json::to_vec(&state).unwrap();
    let mut wire = b"FST2".to_vec();
    wire.extend_from_slice(&1u16.to_le_bytes());
    wire.extend_from_slice(&0u16.to_le_bytes());
    wire.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    wire.extend_from_slice(&payload);
    assert!(crate::server::native_snapshot::decode_state(&wire).is_err());
    assert!(!original.restore_binary_snapshot(&wire));
    assert_eq!(original.capture_terminal_snapshot(), before);
}
