//! Backend for displaying jellyfin rich presence on discord

/// Main module
pub mod core;
/// Useful imports
///
/// Contains imports that most programs will be using.
pub mod prelude;
/// External connections
pub mod services;
pub use crate::core::error;
use colored::Colorize;
pub use core::rpc::setactivity;
pub use discord_rich_presence;
use discord_rich_presence::DiscordIpc;
use discord_rich_presence::DiscordIpcClient;
use log::info;
use retry::retry_with_index;
#[cfg(test)]
mod tests;

/// Function for connecting to the Discord Ipc.
pub fn connect(rich_presence_client: &mut DiscordIpcClient) {
    retry_with_index(
        retry::delay::Exponential::from_millis(1000),
        |current_try| {
            info!(
                "{} {}{}",
                "Attempt".bold().truecolor(225, 69, 0),
                current_try.to_string().bold().truecolor(225, 69, 0),
                ": Trying to connect".bold().truecolor(225, 69, 0)
            );
            match rich_presence_client.connect() {
                Ok(result) => retry::OperationResult::Ok(result),
                Err(err) => {
                    log::error!(
                        "{}\nError: {}",
                        "Failed to connect, retrying soon".red().bold(),
                        err
                    );
                    retry::OperationResult::Retry(())
                }
            }
        },
    )
    .unwrap();
}

/// Built in reqwest::get() function, has an extra field to specify if the self signed cert should be accepted.
pub async fn get<U: reqwest::IntoUrl>(
    url: U,
    self_signed_cert: bool,
) -> Result<reqwest::Response, reqwest::Error> {
    reqwest::Client::builder()
        .danger_accept_invalid_certs(self_signed_cert)
        .build()?
        .get(url)
        .send()
        .await
}
