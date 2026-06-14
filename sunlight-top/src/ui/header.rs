use crate::telemetry::SystemSnapshot;
use crate::terminal::Canvas;

pub fn render_header(c: &mut Canvas, snap: &SystemSnapshot, term_width: u16) {
    c.move_to(1, 1);
    c.bg_surface();
    c.fg_orange();
    c.bold();

    let title = " ☀ sunlight-top ";
    let width = term_width as usize;
    let pad = width.saturating_sub(title.len()) / 2;
    for _ in 0..pad {
        c.push_str("─");
    }
    c.push_str(title);
    let used = pad + title.len();
    for _ in used..width {
        c.push_str("─");
    }
    c.reset();
    c.clear_eol();

    c.move_to(2, 1);
    c.fg_dim();
    c.push_str(" uptime: ");
    c.reset();
    render_uptime(c, snap.uptime_secs);
    c.fg_dim();
    c.push_str("  tasks: ");
    c.reset();
    c.fg_white();
    c.push_u64(snap.proc_count as u64);
    c.reset();
    if snap.local_time_len > 0 {
        c.fg_dim();
        c.push_str("  local: ");
        c.reset();
        c.push_bytes(&snap.local_time[..snap.local_time_len]);
    }
    c.clear_eol();

    c.move_to(3, 1);
    c.fg_dim();
    c.push_str(" CPU  [");
    c.reset();
    c.progress_bar(snap.cpu_usage_pct, term_width.saturating_sub(14));
    c.fg_white();
    c.push_str("] ");
    push_pct(c, snap.cpu_usage_pct);
    c.reset();
    c.clear_eol();

    c.move_to(4, 1);
    let mem_pct = if snap.total_ram_kb > 0 {
        ((snap.used_ram_kb.saturating_mul(100)) / snap.total_ram_kb).min(100) as u8
    } else {
        0
    };
    c.fg_dim();
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
        c.fg_dim();
        c.push_str(" ZRAM [");
        c.reset();
        c.progress_bar(zram_pct, term_width.saturating_sub(14));
        c.push_str("] ");
        push_kb_human(c, snap.zram_comp_kb);
        c.fg_dim();
        c.push_str(" → ");
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
        c.fg_dim();
        c.push_str(" ZRAM  no compressed memory");
        c.reset();
    }
    c.clear_eol();

    c.move_to(6, 1);
    c.fg_dim();
    c.push_str(" NET   ↓ ");
    c.reset();
    push_bytes_human(c, snap.net_rx_bytes);
    c.fg_dim();
    c.push_str("  ↑ ");
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
