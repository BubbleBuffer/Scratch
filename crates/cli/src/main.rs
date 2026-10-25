fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut out = std::io::stdout();
    if let Err(error) = scratch_cli::run(&args, &mut out) {
        eprintln!("scratch-cli: {error:#}");
        std::process::exit(1);
    }
}
