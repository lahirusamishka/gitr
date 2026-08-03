mod app;
mod commit;
mod config;
mod diff;

fn print_help() {
    println!("gitr — a compact git commit graph viewer\n");
    println!("Alternative to gitk, inspired by VS Code Git Graph.");
    println!("Run from any git repository root.\n");
    println!("USAGE:");
    println!("  gitr [path] [--limit N] [--current] [--foreground] [--help]\n");
    println!("OPTIONS:");
    println!("  path          repository path (default: current directory)");
    println!("  --limit N     max commits to load (default: 1000)");
    println!("  --current     only walk the current branch (default: all refs)");
    println!("  --foreground  run in foreground (don't detach from terminal)");
    println!("  --help        show this message\n");
    println!("CONTROLS:");
    println!("  Ctrl+Q      exit");
    println!("  Click row   select commit & show diff");
    println!("  Search      find commits by message, author, or hash");
}

fn daemonize() {
    if cfg!(unix) && std::env::var("GITR_DETACHED").is_err() {
        let args: Vec<String> = std::env::args().collect();
        if let Ok(exe) = std::env::current_exe() {
            if std::process::Command::new(&exe)
                .args(&args[1..])
                .env("GITR_DETACHED", "1")
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .is_ok()
            {
                std::process::exit(0);
            }
        }
    }
}

fn main() -> eframe::Result<()> {
    std::panic::set_hook(Box::new(|info| {
        eprintln!("gitr crashed: {info}");
        eprintln!("Press Ctrl+C to close this terminal.");
    }));

    let mut path = String::from(".");
    let mut limit: usize = 1000;
    let mut all_refs = true;
    let mut foreground = false;

    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--help" | "-h" => {
                print_help();
                std::process::exit(0);
            }
            "--limit" => {
                i += 1;
                if let Some(v) = args.get(i) {
                    limit = v.parse().unwrap_or(1000);
                }
            }
            "--current" => all_refs = false,
            "--foreground" => foreground = true,
            other => path = other.to_string(),
        }
        i += 1;
    }

    if !foreground {
        daemonize();
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1200.0, 800.0]),
        ..Default::default()
    };

    eframe::run_native(
        "gitr",
        options,
        Box::new(move |_cc| Ok(Box::new(app::App::new(path, limit, all_refs)))),
    )
}
