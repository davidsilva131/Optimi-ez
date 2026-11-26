#![windows_subsystem = "windows"]

fn main() {
    let config = match optimi_worker::Config::parse(std::env::args().skip(1)) {
        Ok(config) => config,
        Err(_) => std::process::exit(2),
    };
    if optimi_worker::run(config).is_err() {
        std::process::exit(1);
    }
}
