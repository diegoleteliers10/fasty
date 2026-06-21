use criterion::{black_box, criterion_group, criterion_main, Criterion};
use fasty::parser::VtParser;

fn bench_parser_plain_text(c: &mut Criterion) {
    let mut parser = VtParser::new();
    // 10 KB of plain text
    let data = "hello world\n".repeat(1_000);
    let bytes = data.as_bytes();

    c.bench_function("parser_plain_text", |b| {
        b.iter(|| {
            let actions = parser.feed_str(black_box(bytes));
            black_box(actions);
        })
    });
}

fn bench_parser_sgr_ansi(c: &mut Criterion) {
    let mut parser = VtParser::new();
    // 10 KB of ANSI escape sequences (SGR style)
    let data = "\x1b[31mred\x1b[0m\x1b[1;32mgreen\x1b[0m\n".repeat(400);
    let bytes = data.as_bytes();

    c.bench_function("parser_sgr_ansi", |b| {
        b.iter(|| {
            let actions = parser.feed_str(black_box(bytes));
            black_box(actions);
        })
    });
}

fn bench_parser_complex(c: &mut Criterion) {
    let mut parser = VtParser::new();
    // Mixture of cursor movements, SGR, plain text, and line feeds
    let data = "\x1b[2J\x1b[H\x1b[34m[fasty]\x1b[0m loading...\n\x1b[10;20Hprogress \x1b[32m[====>    ]\x1b[0m 50%\r".repeat(100);
    let bytes = data.as_bytes();

    c.bench_function("parser_complex", |b| {
        b.iter(|| {
            let actions = parser.feed_str(black_box(bytes));
            black_box(actions);
        })
    });
}

criterion_group!(benches, bench_parser_plain_text, bench_parser_sgr_ansi, bench_parser_complex);
criterion_main!(benches);
