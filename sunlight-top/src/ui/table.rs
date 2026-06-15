use crate::telemetry::{ProcessSnapshot, ProcessState, SystemSnapshot};
use crate::terminal::Canvas;

#[derive(Clone, Copy)]
pub struct SortKey {
    pub column: SortColumn,
    pub descending: bool,
}

#[derive(Clone, Copy)]
pub enum SortColumn {
    Pid,
    Cpu,
    Mem,
    Name,
}

pub fn render_table_header(c: &mut Canvas, row: u16) {
    c.move_to(row, 1);
    c.bg_surface();
    c.fg_orange();
    c.bold();
    c.push_str(" PID   PPID  STATE      CPU%  MEM      NAME");
    c.reset();
    c.clear_eol();
}

pub fn render_table(
    c: &mut Canvas,
    snap: &SystemSnapshot,
    start_row: u16,
    max_rows: u16,
    term_width: u16,
    sort: &SortKey,
    my_pid: u32,
) {
    let mut order = [0usize; 64];
    for i in 0..snap.proc_count {
        order[i] = i;
    }
    sort_order(&mut order[..snap.proc_count], snap, sort);

    let used_cols = 6 + 6 + 9 + 6 + 8 + 4;
    let name_col_width = (term_width as usize).saturating_sub(used_cols);

    let visible = core::cmp::min(snap.proc_count, max_rows as usize);
    for row_idx in 0..visible {
        let pi = order[row_idx];
        let proc = &snap.procs[pi];

        c.move_to(start_row + row_idx as u16, 1);

        let is_self = proc.pid == my_pid;
        if is_self {
            c.bg_selected();
        }

        c.fg_dim();
        push_right_aligned_u32(c, proc.pid, 5);
        c.push(b' ');
        push_right_aligned_u32(c, proc.ppid, 5);
        c.push(b' ');

        let state_str = match proc.state {
            ProcessState::Running => "running  ",
            ProcessState::Ready => "ready    ",
            ProcessState::Blocked => "blocked  ",
            ProcessState::Finished => "done     ",
        };

        match proc.state {
            ProcessState::Running => c.fg_green(),
            ProcessState::Blocked => c.fg_yellow(),
            ProcessState::Finished => c.fg_dim(),
            ProcessState::Ready => c.fg_white(),
        }
        c.push_str(state_str);

        match proc.cpu_pct {
            85..=u8::MAX => c.fg_red(),
            60..=84 => c.fg_yellow(),
            _ => c.fg_green(),
        }
        push_right_aligned_u32(c, proc.cpu_pct as u32, 3);
        c.push(b'%');
        c.push(b' ');

        c.fg_white();
        push_kb_human_short(c, proc.mem_kb as u64, 7);
        c.push(b' ');

        if is_self {
            c.fg_orange();
        } else {
            c.fg_white();
        }
        let name = proc.name_str();
        if name.len() > name_col_width && name_col_width > 1 {
            c.push_bytes(&name.as_bytes()[..name_col_width - 1]);
            c.push(b'~');
        } else {
            c.push_padded(name, name_col_width);
        }

        c.reset();
        c.clear_eol();
    }

    for row_idx in visible..(max_rows as usize) {
        c.move_to(start_row + row_idx as u16, 1);
        c.clear_eol();
    }
}

fn sort_order(order: &mut [usize], snap: &SystemSnapshot, sort: &SortKey) {
    for i in 1..order.len() {
        let key = order[i];
        let mut j = i;
        while j > 0 {
            let a = &snap.procs[key];
            let b = &snap.procs[order[j - 1]];
            let before = match sort.column {
                SortColumn::Pid => cmp_u32(a.pid, b.pid, sort.descending),
                SortColumn::Cpu => cmp_u8(a.cpu_pct, b.cpu_pct, sort.descending),
                SortColumn::Mem => cmp_u32(a.mem_kb, b.mem_kb, sort.descending),
                SortColumn::Name => cmp_name(a, b, sort.descending),
            };
            if before {
                order[j] = order[j - 1];
                j -= 1;
            } else {
                break;
            }
        }
        order[j] = key;
    }
}

fn cmp_u32(a: u32, b: u32, descending: bool) -> bool {
    if descending { a > b } else { a < b }
}

fn cmp_u8(a: u8, b: u8, descending: bool) -> bool {
    if descending { a > b } else { a < b }
}

fn cmp_name(a: &ProcessSnapshot, b: &ProcessSnapshot, descending: bool) -> bool {
    let na = a.name_str();
    let nb = b.name_str();
    if descending {
        na > nb
    } else {
        na < nb
    }
}

fn push_right_aligned_u32(c: &mut Canvas, v: u32, width: usize) {
    let mut tmp = [0u8; 10];
    let mut n = 0usize;
    let mut x = v;

    if x == 0 {
        tmp[0] = b'0';
        n = 1;
    } else {
        while x > 0 {
            tmp[n] = b'0' + (x % 10) as u8;
            n += 1;
            x /= 10;
        }
    }

    for _ in n..width {
        c.push(b' ');
    }
    while n > 0 {
        n -= 1;
        c.push(tmp[n]);
    }
}

fn push_kb_human_short(c: &mut Canvas, kb: u64, width: usize) {
    let mut tmp = [0u8; 16];
    let mut len = 0usize;

    if kb >= 1024 * 1024 {
        len += push_dec_into(&mut tmp[len..], kb / (1024 * 1024));
        tmp[len] = b'G';
        len += 1;
    } else if kb >= 1024 {
        len += push_dec_into(&mut tmp[len..], kb / 1024);
        tmp[len] = b'M';
        len += 1;
    } else {
        len += push_dec_into(&mut tmp[len..], kb);
        tmp[len] = b'K';
        len += 1;
    }

    for _ in len..width {
        c.push(b' ');
    }
    c.push_bytes(&tmp[..len]);
}

fn push_dec_into(out: &mut [u8], mut v: u64) -> usize {
    if v == 0 {
        out[0] = b'0';
        return 1;
    }

    let mut rev = [0u8; 20];
    let mut n = 0usize;
    while v > 0 {
        rev[n] = b'0' + (v % 10) as u8;
        n += 1;
        v /= 10;
    }

    for i in 0..n {
        out[i] = rev[n - 1 - i];
    }
    n
}
