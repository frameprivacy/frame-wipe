//! Human-readable sizes and durations.

pub fn format_bytes(n: u64) -> String {
    const KIB: f64 = 1024.0;
    let n = n as f64;
    if n < KIB {
        format!("{n:.0} B")
    } else if n < KIB * KIB {
        format!("{:.1} KiB", n / KIB)
    } else if n < KIB * KIB * KIB {
        format!("{:.2} MiB", n / (KIB * KIB))
    } else if n < KIB * KIB * KIB * KIB {
        format!("{:.2} GiB", n / (KIB * KIB * KIB))
    } else {
        format!("{:.2} TiB", n / (KIB * KIB * KIB * KIB))
    }
}

pub fn format_throughput(bytes_per_sec: u64) -> String {
    format!("{}/s", format_bytes(bytes_per_sec))
}

pub fn format_eta(secs: u64) -> String {
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    if h > 0 {
        format!("{h}h {m:02}m {s:02}s")
    } else if m > 0 {
        format!("{m}m {s:02}s")
    } else {
        format!("{s}s")
    }
}

/// Parse values like "8M", "1MiB", "4096", "8MB".
pub fn parse_size(s: &str) -> Result<u64, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("empty size".into());
    }
    let (num, rest) = s
        .char_indices()
        .find(|(_, c)| !c.is_ascii_digit() && *c != '.')
        .map(|(i, _)| s.split_at(i))
        .unwrap_or((s, ""));
    let value: f64 = num
        .parse()
        .map_err(|_| format!("invalid size number: {s}"))?;
    if !value.is_finite() || value < 0.0 {
        return Err(format!("invalid size number: {s}"));
    }
    let unit = rest.trim().to_ascii_lowercase();
    let mul: u64 = match unit.as_str() {
        "" | "b" => 1,
        "k" | "kb" | "kib" => 1024,
        "m" | "mb" | "mib" => 1024 * 1024,
        "g" | "gb" | "gib" => 1024 * 1024 * 1024,
        _ => return Err(format!("unknown size unit in {s}")),
    };
    Ok((value * mul as f64).round() as u64)
}

pub fn align_up(n: u64, align: u64) -> u64 {
    if align == 0 {
        return n;
    }
    let rem = n % align;
    if rem == 0 {
        n
    } else {
        n + (align - rem)
    }
}

pub fn align_down(n: u64, align: u64) -> u64 {
    if align == 0 {
        n
    } else {
        n - (n % align)
    }
}
