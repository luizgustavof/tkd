// sem console extra no Windows em release
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // `tkd-app --export-bindings <arquivo.ts>`: so gera os tipos do front e sai
    let args: Vec<String> = std::env::args().collect();
    if let Some(i) = args.iter().position(|a| a == "--export-bindings") {
        let path = args.get(i + 1).cloned().unwrap_or_else(|| "../src/bindings.ts".into());
        match tkd_app_lib::export_bindings(&path) {
            Ok(()) => println!("bindings em {path}"),
            Err(e) => {
                eprintln!("{e:#}");
                std::process::exit(1);
            }
        }
        return;
    }
    tkd_app_lib::run()
}
