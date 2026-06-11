/// SunlightOS File Utilities - Busybox-style dispatcher
/// Usage: sunlight-utils <command> [args...]
/// Symlinks (ls, cat, cp, etc.) point to this binary

// Stub implementation for Phase 5.x testing

fn main() {
    let args: Vec<String> = env::args().collect();
    let cmd_name = if args.is_empty() {
        "unknown"
    } else {
        let path = Path::new(&args[0]);
        path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
    };

    let args_slice = if args.len() > 1 { &args[1..] } else { &[] };

    let exit_code = match cmd_name {
        "ls" => cmd_ls(args_slice),
        "cat" => cmd_cat(args_slice),
        "cp" => cmd_cp(args_slice),
        "mv" => cmd_mv(args_slice),
        "rm" => cmd_rm(args_slice),
        "mkdir" => cmd_mkdir(args_slice),
        "rmdir" => cmd_rmdir(args_slice),
        "touch" => cmd_touch(args_slice),
        "chmod" => cmd_chmod(args_slice),
        "chown" => cmd_chown(args_slice),
        "find" => cmd_find(args_slice),
        "grep" => cmd_grep(args_slice),
        "head" => cmd_head(args_slice),
        "tail" => cmd_tail(args_slice),
        "wc" => cmd_wc(args_slice),
        "sort" => cmd_sort(args_slice),
        "uniq" => cmd_uniq(args_slice),
        "cut" => cmd_cut(args_slice),
        "file" => cmd_file(args_slice),
        "stat" => cmd_stat(args_slice),
        "pwd" => cmd_pwd(args_slice),
        "cd" => cmd_cd(args_slice),
        "echo" => cmd_echo(args_slice),
        "date" => cmd_date(args_slice),
        _ => {
            eprintln!("sunlight-utils: command '{}' not found", cmd_name);
            127
        }
    };

    std::process::exit(exit_code);
}

// File listing
fn cmd_ls(args: &[String]) -> i32 {
    let path = if args.is_empty() { "." } else { &args[0] };
    match fs::read_dir(path) {
        Ok(entries) => {
            for entry in entries {
                if let Ok(entry) = entry {
                    if let Some(name) = entry.file_name().to_str() {
                        println!("{}", name);
                    }
                }
            }
            0
        }
        Err(e) => {
            eprintln!("ls: {}: {}", path, e);
            1
        }
    }
}

// File reading
fn cmd_cat(args: &[String]) -> i32 {
    if args.is_empty() {
        eprintln!("cat: missing file operand");
        return 1;
    }

    for arg in args {
        match fs::read(arg) {
            Ok(contents) => {
                if let Ok(text) = String::from_utf8(contents) {
                    print!("{}", text);
                } else {
                    eprint!("cat: {} contains invalid UTF-8", arg);
                    return 1;
                }
            }
            Err(e) => {
                eprintln!("cat: {}: {}", arg, e);
                return 1;
            }
        }
    }
    0
}

// File copying
fn cmd_cp(args: &[String]) -> i32 {
    if args.len() < 2 {
        eprintln!("cp: missing file operand");
        return 1;
    }

    match fs::copy(&args[0], &args[1]) {
        Ok(_) => 0,
        Err(e) => {
            eprintln!("cp: {}: {}", args[0], e);
            1
        }
    }
}

// File moving
fn cmd_mv(args: &[String]) -> i32 {
    if args.len() < 2 {
        eprintln!("mv: missing file operand");
        return 1;
    }

    match fs::rename(&args[0], &args[1]) {
        Ok(_) => 0,
        Err(e) => {
            eprintln!("mv: {}: {}", args[0], e);
            1
        }
    }
}

// File removal
fn cmd_rm(args: &[String]) -> i32 {
    if args.is_empty() {
        eprintln!("rm: missing file operand");
        return 1;
    }

    for arg in args {
        match fs::remove_file(arg) {
            Ok(_) => {}
            Err(e) => {
                eprintln!("rm: {}: {}", arg, e);
                return 1;
            }
        }
    }
    0
}

// Directory creation
fn cmd_mkdir(args: &[String]) -> i32 {
    if args.is_empty() {
        eprintln!("mkdir: missing operand");
        return 1;
    }

    for arg in args {
        match fs::create_dir(arg) {
            Ok(_) => {}
            Err(e) => {
                eprintln!("mkdir: {}: {}", arg, e);
                return 1;
            }
        }
    }
    0
}

// Directory removal
fn cmd_rmdir(args: &[String]) -> i32 {
    if args.is_empty() {
        eprintln!("rmdir: missing operand");
        return 1;
    }

    for arg in args {
        match fs::remove_dir(arg) {
            Ok(_) => {}
            Err(e) => {
                eprintln!("rmdir: {}: {}", arg, e);
                return 1;
            }
        }
    }
    0
}

// File touching (create empty or update timestamp)
fn cmd_touch(args: &[String]) -> i32 {
    if args.is_empty() {
        eprintln!("touch: missing file operand");
        return 1;
    }

    for arg in args {
        if !Path::new(arg).exists() {
            match fs::File::create(arg) {
                Ok(_) => {}
                Err(e) => {
                    eprintln!("touch: {}: {}", arg, e);
                    return 1;
                }
            }
        }
    }
    0
}

// Permission changing (stub)
fn cmd_chmod(args: &[String]) -> i32 {
    if args.len() < 2 {
        eprintln!("chmod: missing operand");
        return 1;
    }
    // Not implemented on most file systems without kernel support
    println!("chmod: mode {} on {} (not supported)", args[0], args[1]);
    0
}

// Owner changing (stub)
fn cmd_chown(args: &[String]) -> i32 {
    if args.len() < 2 {
        eprintln!("chown: missing operand");
        return 1;
    }
    // Not implemented on most file systems without kernel support
    println!("chown: owner {} on {} (not supported)", args[0], args[1]);
    0
}

// Find files (simple recursive search)
fn cmd_find(args: &[String]) -> i32 {
    let path = if args.is_empty() { "." } else { &args[0] };
    let pattern = if args.len() > 2 && args[1] == "-name" {
        Some(&args[2])
    } else {
        None
    };

    fn search(path: &Path, pattern: Option<&String>) -> io::Result<()> {
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            let path = entry.path();
            let name = entry.file_name();
            let name_str = name.to_string_lossy();

            let matches = if let Some(p) = pattern {
                name_str.contains(p)
            } else {
                true
            };

            if matches {
                println!("{}", path.display());
            }

            if path.is_dir() {
                let _ = search(&path, pattern);
            }
        }
        Ok(())
    }

    match search(Path::new(path), pattern) {
        Ok(_) => 0,
        Err(e) => {
            eprintln!("find: {}: {}", path, e);
            1
        }
    }
}

// Grep (simple pattern search)
fn cmd_grep(args: &[String]) -> i32 {
    if args.len() < 2 {
        eprintln!("grep: missing operand");
        return 1;
    }

    let pattern = &args[0];
    for file_arg in &args[1..] {
        match fs::read_to_string(file_arg) {
            Ok(contents) => {
                for line in contents.lines() {
                    if line.contains(pattern) {
                        println!("{}", line);
                    }
                }
            }
            Err(e) => {
                eprintln!("grep: {}: {}", file_arg, e);
                return 1;
            }
        }
    }
    0
}

// Head (print first N lines)
fn cmd_head(args: &[String]) -> i32 {
    let (lines, file) = if args.len() >= 2 && args[0] == "-n" {
        (args[1].parse::<usize>().unwrap_or(10), &args[2])
    } else if args.is_empty() {
        eprintln!("head: missing file");
        return 1;
    } else {
        (10, &args[0])
    };

    match fs::read_to_string(file) {
        Ok(contents) => {
            for (i, line) in contents.lines().enumerate() {
                if i >= lines {
                    break;
                }
                println!("{}", line);
            }
            0
        }
        Err(e) => {
            eprintln!("head: {}: {}", file, e);
            1
        }
    }
}

// Tail (print last N lines)
fn cmd_tail(args: &[String]) -> i32 {
    let (lines, file) = if args.len() >= 2 && args[0] == "-n" {
        (args[1].parse::<usize>().unwrap_or(10), &args[2])
    } else if args.is_empty() {
        eprintln!("tail: missing file");
        return 1;
    } else {
        (10, &args[0])
    };

    match fs::read_to_string(file) {
        Ok(contents) => {
            let all_lines: Vec<&str> = contents.lines().collect();
            let start = if all_lines.len() > lines {
                all_lines.len() - lines
            } else {
                0
            };
            for line in &all_lines[start..] {
                println!("{}", line);
            }
            0
        }
        Err(e) => {
            eprintln!("tail: {}: {}", file, e);
            1
        }
    }
}

// Word count
fn cmd_wc(args: &[String]) -> i32 {
    if args.is_empty() {
        eprintln!("wc: missing file");
        return 1;
    }

    for file in args {
        match fs::read_to_string(file) {
            Ok(contents) => {
                let lines = contents.lines().count();
                let words = contents.split_whitespace().count();
                let bytes = contents.len();
                println!("  {} {} {} {}", lines, words, bytes, file);
            }
            Err(e) => {
                eprintln!("wc: {}: {}", file, e);
                return 1;
            }
        }
    }
    0
}

// Sort lines
fn cmd_sort(args: &[String]) -> i32 {
    if args.is_empty() {
        eprintln!("sort: missing file");
        return 1;
    }

    match fs::read_to_string(&args[0]) {
        Ok(contents) => {
            let mut lines: Vec<&str> = contents.lines().collect();
            lines.sort();
            for line in lines {
                println!("{}", line);
            }
            0
        }
        Err(e) => {
            eprintln!("sort: {}: {}", args[0], e);
            1
        }
    }
}

// Unique lines
fn cmd_uniq(args: &[String]) -> i32 {
    if args.is_empty() {
        eprintln!("uniq: missing file");
        return 1;
    }

    match fs::read_to_string(&args[0]) {
        Ok(contents) => {
            let mut prev = "";
            for line in contents.lines() {
                if line != prev {
                    println!("{}", line);
                    prev = line;
                }
            }
            0
        }
        Err(e) => {
            eprintln!("uniq: {}: {}", args[0], e);
            1
        }
    }
}

// Cut fields (simple CSV)
fn cmd_cut(args: &[String]) -> i32 {
    eprintln!("cut: not implemented");
    1
}

// File type
fn cmd_file(args: &[String]) -> i32 {
    if args.is_empty() {
        eprintln!("file: missing file");
        return 1;
    }

    for arg in args {
        match fs::metadata(arg) {
            Ok(meta) => {
                let type_str = if meta.is_dir() {
                    "directory"
                } else if meta.is_file() {
                    "regular file"
                } else {
                    "unknown"
                };
                println!("{}: {}", arg, type_str);
            }
            Err(e) => {
                eprintln!("file: {}: {}", arg, e);
                return 1;
            }
        }
    }
    0
}

// File stats
fn cmd_stat(args: &[String]) -> i32 {
    if args.is_empty() {
        eprintln!("stat: missing file");
        return 1;
    }

    for arg in args {
        match fs::metadata(arg) {
            Ok(meta) => {
                println!("File: {}", arg);
                println!("Size: {} bytes", meta.len());
                println!("Is dir: {}", meta.is_dir());
                println!("Is file: {}", meta.is_file());
            }
            Err(e) => {
                eprintln!("stat: {}: {}", arg, e);
                return 1;
            }
        }
    }
    0
}

// Print working directory
fn cmd_pwd(_args: &[String]) -> i32 {
    match env::current_dir() {
        Ok(path) => {
            println!("{}", path.display());
            0
        }
        Err(e) => {
            eprintln!("pwd: {}", e);
            1
        }
    }
}

// Change directory (limited support)
fn cmd_cd(args: &[String]) -> i32 {
    let path = if args.is_empty() { "/" } else { &args[0] };
    match env::set_current_dir(path) {
        Ok(_) => 0,
        Err(e) => {
            eprintln!("cd: {}: {}", path, e);
            1
        }
    }
}

// Echo
fn cmd_echo(args: &[String]) -> i32 {
    println!("{}", args.join(" "));
    0
}

// Date
fn cmd_date(_args: &[String]) -> i32 {
    println!("Wed Jun 11 2026 12:00:00 UTC");
    0
}
