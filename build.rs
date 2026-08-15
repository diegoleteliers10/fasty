use std::collections::BTreeSet;
use std::io::Write;

fn main() {
    // Windows icon resource
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winres::WindowsResource::new();
        res.set_icon("assets/fasttyIcon.ico");
        res.compile().unwrap();
    }

    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR not set");

    // --- Generate sextant mask table from UnicodeData.txt ---
    let ucdata_path = std::path::Path::new("data").join("UnicodeData.txt");
    if ucdata_path.exists() {
        let ucdata = std::fs::read_to_string(&ucdata_path).expect("Failed to read data/UnicodeData.txt");
        let mut sextant_table = [0u8; 60];
        for line in ucdata.lines() {
            let fields: Vec<&str> = line.split(';').collect();
            if fields.len() < 2 {
                continue;
            }
            let codepoint = u32::from_str_radix(fields[0], 16).ok();
            let Some(cp) = codepoint else { continue };
            if !(0x1FB00..=0x1FB3B).contains(&cp) {
                continue;
            }
            let name = fields[1];
            if !name.starts_with("BLOCK SEXTANT-") {
                continue;
            }
            let digits = name.strip_prefix("BLOCK SEXTANT-").unwrap();
            let mut mask = 0u8;
            for ch in digits.chars() {
                if let Some(d) = ch.to_digit(10) {
                    if (1..=6).contains(&d) {
                        mask |= 1 << (d - 1);
                    }
                }
            }
            let idx = (cp - 0x1FB00) as usize;
            sextant_table[idx] = mask;
        }

        let mut sext = std::fs::File::create(
            std::path::Path::new(&out_dir).join("sextant_table.rs"),
        ).expect("Failed to create sextant_table.rs");
        writeln!(sext, "// Auto-generated from UnicodeData.txt (BLOCK SEXTANT entries)").ok();
        writeln!(sext, "#[allow(non_upper_case_globals)]").ok();
        write!(sext, "const SEXTANT_MASK_TABLE: [u8; 60] = [").ok();
        for (i, &val) in sextant_table.iter().enumerate() {
            if i % 10 == 0 {
                write!(sext, "\n    ").ok();
            }
            write!(sext, "0x{:02x}, ", val).ok();
        }
        writeln!(sext, "\n];").ok();
        println!("cargo::rerun-if-changed=data/UnicodeData.txt");
        eprintln!("Generated sextant table with {} entries", sextant_table.len());
    } else {
        eprintln!("WARN: data/UnicodeData.txt not found; skipping sextant table generation");
    }

    // --- Generate emoji table from emoji-data.txt ---
    let emoji_path = std::path::Path::new("data").join("emoji-data.txt");
    if !emoji_path.exists() {
        eprintln!("WARN: data/emoji-data.txt not found; skipping emoji table generation");
        return;
    }
    let content = std::fs::read_to_string(&emoji_path).expect("Failed to read data/emoji-data.txt");
    let out_path = std::path::Path::new(&out_dir).join("emoji_table.rs");

    let mut ranges: BTreeSet<(u32, u32)> = BTreeSet::new();

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let parts: Vec<&str> = line.splitn(2, ';').collect();
        if parts.len() < 2 {
            continue;
        }
        let prop = parts[1].split('#').next().unwrap_or("").trim();
        if prop != "Emoji_Presentation" {
            continue;
        }
        let codepoint_str = parts[0].trim();
        if let Some((a, b)) = codepoint_str.split_once("..") {
            let start = u32::from_str_radix(a.trim(), 16).ok();
            let end = u32::from_str_radix(b.trim(), 16).ok();
            if let (Some(s), Some(e)) = (start, end) {
                ranges.insert((s, e));
            }
        } else {
            if let Ok(cp) = u32::from_str_radix(codepoint_str, 16) {
                ranges.insert((cp, cp));
            }
        }
    }

    // Add special ranges not covered by Emoji_Presentation
    ranges.insert((0x1F1E6, 0x1F1FF)); // Regional Indicator Symbols
    ranges.insert((0xE0000, 0xE007F)); // Tags block

    let merged = merge_ranges(ranges);

    let mut f = std::fs::File::create(&out_path).expect("Failed to create emoji_table.rs");

    writeln!(f, "// Auto-generated from Unicode Emoji data (Emoji_Presentation + special ranges)").ok();
    writeln!(f, "#[allow(non_upper_case_globals)]").ok();
    writeln!(f, "fn generated_is_emoji(codepoint: u32) -> bool {{").ok();
    writeln!(f, "    matches!(codepoint,").ok();
    for (i, (start, end)) in merged.iter().enumerate() {
        let sep = if i + 1 < merged.len() { " |" } else { "" };
        if start == end {
            writeln!(f, "        0x{:04X}{}", start, sep).ok();
        } else {
            writeln!(f, "        0x{:04X}..=0x{:04X}{}", start, end, sep).ok();
        }
    }
    writeln!(f, "    )").ok();
    writeln!(f, "}}").ok();

    println!("cargo::rerun-if-changed=data/emoji-data.txt");
    eprintln!("Generated emoji table with {} range(s)", merged.len());
}

fn merge_ranges(ranges: BTreeSet<(u32, u32)>) -> Vec<(u32, u32)> {
    let mut sorted: Vec<(u32, u32)> = ranges.into_iter().collect();
    sorted.sort();

    let mut merged: Vec<(u32, u32)> = Vec::new();
    for &(s, e) in &sorted {
        if let Some(last) = merged.last_mut() {
            if last.1 + 1 >= s {
                last.1 = last.1.max(e);
                continue;
            }
        }
        merged.push((s, e));
    }
    merged
}
