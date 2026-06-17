use crate::telemetry::SystemSnapshot;
use crate::terminal::Canvas;

pub fn render_header(c: &mut Canvas, snap: &SystemSnapshot, term_width: u16) {
    c.move_to(1, 1);
    c.bg_surface();
    c.fg_orange();
    c.bold();

    // ASCII-only chrome: the TTY grid renders one byte per cell, so box-drawing
    // and other multi-byte glyphs would come out as garbage.
    let title = " * sunlight-top ";
    let width = term_width as usize;
    let pad = width.saturating_sub(title.len()) / 2;
    for _ in 0..pad {
        c.push(b'-');
    }
    c.push_str(title);
    let used = pad + title.len();
    for _ in used..width {
        c.push(b'-');
    }
    c.reset();
    c.clear_eol();

    c.move_to(2, 1);
    c.fg_orange();
    c.push_str(" uptime: ");
    c.reset();
    render_uptime(c, snap.uptime_secs);
    c.fg_orange();
    c.push_str("  tasks: ");
    c.reset();
    c.fg_white();
    c.push_u64(snap.proc_count as u64);
    c.reset();
    if snap.local_time_len > 0 {
        c.fg_orange();
        c.push_str("  local: ");
        c.reset();
        c.push_bytes(&snap.local_time[..snap.local_time_len]);
    }
    c.clear_eol();

    // CPU line: machine-normalized used/idle with two decimals.
    // 10000 basis points = 100.00%. Do not show raw capacity as "100% used".
    c.move_to(3, 1);
    c.fg_orange();
    c.push_str(" CPU: ");
    c.reset();

    // Progress bar driven by used (0..100 coarse).
    let used_pct_coarse = (snap.cpu_used_bp as u32 / 100) as u8;
    c.progress_bar(used_pct_coarse, term_width.saturating_sub(28));

    c.fg_white();
    // used: AB.CD
    push_bp_as_pct(c, snap.cpu_used_bp);
    c.reset();
    c.fg_dim();
    c.push_str(" used, ");
    c.reset();
    // idle: AB.CD
    push_bp_as_pct(c, snap.cpu_idle_bp);
    c.fg_dim();
    c.push_str(" idle");
    c.reset();

    // Show CPU count and normalization note.
    c.fg_dim();
    c.push_str("  cpus:");
    c.reset();
    c.fg_white();
    c.push_u64(snap.cpu_count as u64);
    c.reset();
    c.clear_eol();

    c.move_to(4, 1);
    let mem_pct = if snap.total_ram_kb > 0 {
        ((snap.used_ram_kb.saturating_mul(100)) / snap.total_ram_kb).min(100) as u8
    } else {
        0
    };
    c.fg_orange();
    c.push_str(" MEM  [");
    c.reset();
    c.progress_bar(mem_pct, term_width.saturating_sub(14));
    c.push_str("] ");
    push_kb_human(c, snap.used_ram_kb);
    c.fg_dim();
    c.push_str(" / ");
    c.reset();
    push_kb_human(c, snap.total_ram_kb);
    c.fg_dim();
    c.push_str(" (");
    c.reset();
    push_pct(c, mem_pct);
    c.fg_dim();
    c.push(b')');
    c.reset();
    c.clear_eol();

    c.move_to(5, 1);
    if snap.zram_orig_kb > 0 {
        let zram_pct = ((snap.zram_comp_kb.saturating_mul(100)) / snap.zram_orig_kb.max(1)).min(100) as u8;
        let ratio = snap.zram_orig_kb / snap.zram_comp_kb.max(1);
        c.fg_orange();
        c.push_str(" ZRAM [");
        c.reset();
        c.progress_bar(zram_pct, term_width.saturating_sub(14));
        c.push_str("] ");
        push_kb_human(c, snap.zram_comp_kb);
        c.fg_dim();
        c.push_str(" -> ");
        c.reset();
        push_kb_human(c, snap.zram_orig_kb);
        c.fg_dim();
        c.push_str(" (ratio ");
        c.reset();
        c.push_u64(ratio);
        c.fg_dim();
        c.push_str("x)");
        c.reset();
    } else {
        c.fg_orange();
        c.push_str(" ZRAM ");
        c.reset();
        c.fg_dim();
        c.push_str(" no compressed memory");
        c.reset();
    }
    c.clear_eol();

    c.move_to(6, 1);
    c.fg_orange();
    c.push_str(" NET   Rx ");
    c.reset();
    push_bytes_human(c, snap.net_rx_bytes);
    c.fg_orange();
    c.push_str("  Tx ");
    c.reset();
    push_bytes_human(c, snap.net_tx_bytes);
    c.fg_dim();
    c.push_str("  (eth0)");
    c.reset();
    c.clear_eol();
}

pub fn push_kb_human(c: &mut Canvas, kb: u64) {
    if kb >= 1024 * 1024 {
        let g_whole = kb / (1024 * 1024);
        let g_tenths = ((kb % (1024 * 1024)) * 10) / (1024 * 1024);
        c.push_u64(g_whole);
        c.push(b'.');
        c.push(b'0' + (g_tenths as u8));
        c.push(b'G');
    } else if kb >= 1024 {
        let m_whole = kb / 1024;
        let m_tenths = ((kb % 1024) * 10) / 1024;
        c.push_u64(m_whole);
        c.push(b'.');
        c.push(b'0' + (m_tenths as u8));
        c.push(b'M');
    } else {
        c.push_u64(kb);
        c.push(b'K');
    }
}

pub fn push_bytes_human(c: &mut Canvas, bytes: u64) {
    if bytes >= 1024 * 1024 {
        let m_whole = bytes / (1024 * 1024);
        let m_tenths = ((bytes % (1024 * 1024)) * 10) / (1024 * 1024);
        c.push_u64(m_whole);
        c.push(b'.');
        c.push(b'0' + (m_tenths as u8));
        c.push_str(" MB");
    } else if bytes >= 1024 {
        let k_whole = bytes / 1024;
        let k_tenths = ((bytes % 1024) * 10) / 1024;
        c.push_u64(k_whole);
        c.push(b'.');
        c.push(b'0' + (k_tenths as u8));
        c.push_str(" KB");
    } else {
        c.push_u64(bytes);
        c.push_str(" B");
    }
}

pub fn render_uptime(c: &mut Canvas, secs: u64) {
    let days = secs / 86_400;
    let rem = secs % 86_400;
    let h = rem / 3600;
    let rem2 = rem % 3600;
    let m = rem2 / 60;
    let s = rem2 % 60;

    if days > 0 {
        c.push_u64(days);
        c.push(b'd');
        c.push(b' ');
    }

    push_two(c, h);
    c.push(b':');
    push_two(c, m);
    c.push(b':');
    push_two(c, s);
}

fn push_two(c: &mut Canvas, v: u64) {
    c.push(b'0' + ((v / 10) as u8));
    c.push(b'0' + ((v % 10) as u8));
}

pub fn push_pct(c: &mut Canvas, pct: u8) {
    c.push_u64(pct as u64);
    c.push(b'%');
}

/// Render basis points (0..=10000) as two-decimal percent, e.g. 742 -> "7.42".
/// Clamped for safety.
pub fn push_bp_as_pct(c: &mut Canvas, bp: u16) {
    let bp = bp.min(10000);
    let whole = (bp / 100) as u64;
    let hundredths = (bp % 100) as u64;
    c.push_u64(whole);
    c.push(b'.');
    let t = (hundredths / 10) as u8;
    let u = (hundredths % 10) as u8;
    c.push(b'0' + t);
    c.push(b'0' + u);
    c.push(b'%');
}
