mod cli;
mod tui;

#[tokio::main]
async fn main() {
    if let Err(err) = cli::dispatch().await {
        eprintln!("argos: {err:#}");
        std::process::exit(1);
    }
}
