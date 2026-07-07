mod antigravity;

use std::env;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let args: Vec<String> = env::args().collect();

    match args.get(1).map(|s| s.as_str()) {
        Some("login") => match args.get(2).map(|s| s.as_str()) {
            Some("antigravity") => antigravity::login("antigravity").await?,
            Some("gemini-cli") => antigravity::login("gemini-cli").await?,
            Some(provider) => {
                eprintln!("Unknown provider: {provider}");
                eprintln!("Supported providers: antigravity, gemini-cli");
                std::process::exit(1);
            }
            None => {
                eprintln!("Usage: blackrouter-cli login <provider>");
                eprintln!("Supported providers: antigravity, gemini-cli");
                std::process::exit(1);
            }
        },
        Some("--help") | Some("-h") | None => {
            println!("BlackRouter CLI");
            println!();
            println!("USAGE:");
            println!("    blackrouter-cli login <provider>");
            println!();
            println!("PROVIDERS:");
            println!("    antigravity    Google Antigravity (Gemini Code Assist)");
            println!("    gemini-cli     Google Gemini CLI (Gemini API)");
        }
        Some(unknown) => {
            eprintln!("Unknown command: {unknown}");
            eprintln!("Run 'blackrouter-cli --help' for usage.");
            std::process::exit(1);
        }
    }

    Ok(())
}
