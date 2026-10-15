#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

fn main() {
    // Native Messaging stdout is a binary protocol; never log diagnostics or
    // authentication material to stdout/stderr, including panic details.
    std::panic::set_hook(Box::new(|_| {}));
    let result = (|| {
        let executable = std::env::current_exe().map_err(|_| "config_unavailable")?;
        let config_path = executable
            .parent()
            .ok_or("config_unavailable")?
            .join("native-host-config.json");
        let config = director_native_browser_host::Config::read(&config_path)?;
        let arguments: Vec<String> = std::env::args().skip(1).collect();
        let origin = config.validate_arguments(&arguments)?;
        director_native_browser_host::run(std::io::stdin(), std::io::stdout(), config, origin)
    })();
    if result.is_err() {
        std::process::exit(1);
    }
}
