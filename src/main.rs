mod app;
mod commit;
mod config;
mod diff;

fn print_help() {
    println!("rgitk-gui — git commit graph viewer with smooth curved lanes\n");
    println!("USAGE:");
    println!("  rgitk-gui [path] [--limit N] [--current] [--help]\n");
    println!("OPTIONS:");
    println!("  path        repository path (default: current directory)");
    println!("  --limit N   max commits to load (default: 1000)");
    println!("  --current   only walk the current branch (default: all refs)");
    println!("  --help      show this message");
}

fn main() -> eframe::Result<()> {
    let mut path = String::from(".");
    let mut limit: usize = 1000;
    let mut all_refs = true;

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
            other => path = other.to_string(),
        }
        i += 1;
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1200.0, 800.0]),
        ..Default::default()
    };

    eframe::run_native(
        "rgitk-gui",
        options,
        Box::new(move |_cc| Ok(Box::new(app::App::new(path, limit, all_refs)))),
    )
}
