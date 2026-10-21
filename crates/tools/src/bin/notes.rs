fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match vault_tools::notes_cli::run(&args) {
        Ok(output) => println!("{output}"),
        Err(error) => {
            eprintln!("notes: {error}");
            std::process::exit(1);
        }
    }
}
