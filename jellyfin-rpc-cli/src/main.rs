use clap::Parser;
use colored::Colorize;
pub use jellyfin_rpc::core::rpc::show_paused;
use jellyfin_rpc::discord_rich_presence::{activity, DiscordIpc, DiscordIpcClient};
pub use jellyfin_rpc::prelude::*;
pub use jellyfin_rpc::services::imgur::*;
use log::{error, info, warn};
use retry::retry_with_index;
use simple_logger::SimpleLogger;
use time::macros::format_description;
#[cfg(feature = "updates")]
mod updates;

/*
    TODO: Comments
*/

const VERSION: Option<&'static str> = option_env!("CARGO_PKG_VERSION");

#[derive(Parser)]
#[command(author = "Radical <Radiicall> <radical@radical.fun>")]
#[command(version)]
#[command(about = "Rich presence for Jellyfin", long_about = None)]
struct Args {
    #[arg(short = 'c', long = "config", help = "Path to the config file")]
    config: Option<String>,
    #[arg(
        short = 'i',
        long = "image-urls-file",
        help = "Path to image urls file for imgur"
    )]
    image_urls: Option<String>,
    #[arg(
        short = 't',
        long = "wait-time",
        help = "Time to wait between loops in seconds",
        default_value_t = 3
    )]
    wait_time: usize,
    #[arg(
        short = 's',
        long = "suppress-warnings",
        help = "Stops warnings from showing on startup",
        default_value_t = false
    )]
    suppress_warnings: bool,
    #[arg(
        short = 'v',
        long = "log-level",
        help = "Sets the log level to one of: trace, debug, info, warn, error, off",
        default_value_t = String::from("info")
    )]
    log_level: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    if std::env::var("RUST_LOG").is_err() {
        let _ = tokio::task::spawn_blocking(move || {
            std::env::set_var("RUST_LOG", args.log_level);
        })
        .await;
    }

    SimpleLogger::new()
        .with_level(log::LevelFilter::Info)
        .env()
        .with_timestamp_format(format_description!(
            "[year]-[month]-[day] [hour]:[minute]:[second]"
        ))
        .init()
        .unwrap();

    info!("Initializing Jellyfin-RPC");

    #[cfg(feature = "updates")]
    updates::checker().await;

    let config_path = args.config.unwrap_or_else(|| {
        get_config_path().unwrap_or_else(|err| {
            error!("Error determining config path: {:?}", err);
            std::process::exit(1)
        })
    });

    std::fs::create_dir_all(
        std::path::Path::new(&config_path)
            .parent()
            .expect("Invalid config file path"),
    )
    .ok();

    let config = Config::load(&config_path).unwrap_or_else(|e| {
        error!("{} {:?}", "Config can't be loaded:".bold().red(), e);
        error!(
            "{} {}",
            "Config file should be located at:".bold().red(),
            config_path
        );
        std::process::exit(2)
    });

    if !args.suppress_warnings && config.jellyfin.self_signed_cert.is_some_and(|val| val) {
        warn!("{}", "Self-signed certificates are enabled!".bold().red());
    }

    if !args.suppress_warnings
        && config
            .clone()
            .images
            .and_then(|images| images.enable_images)
            .unwrap_or(false)
        && !config
            .clone()
            .images
            .and_then(|images| images.imgur_images)
            .unwrap_or(false)
    {
        warn!(
            "{}",
            "Images without Imgur requires port forwarding!"
                .bold()
                .red()
        )
    }
    if config.jellyfin.blacklist.is_some() {
        let blacklist = config.jellyfin.blacklist.clone().unwrap();
        if let Some(media_types) = blacklist.media_types {
            if !media_types.is_empty() {
                info!(
                    "{} {}",
                    "These media types won't be shown:".bold().red(),
                    media_types
                        .iter()
                        .map(|x| x.to_string())
                        .collect::<Vec<String>>()
                        .join(", ")
                        .bold()
                        .red()
                )
            }
        }

        if let Some(libraries) = blacklist.libraries {
            if !libraries.is_empty() {
                info!(
                    "{} {}",
                    "These media libraries won't be shown:".bold().red(),
                    libraries.join(", ").bold().red()
                )
            }
        }
    }

    let mut connected: bool = false;
    let mut rich_presence_client = DiscordIpcClient::new(
        config
            .discord
            .clone()
            .and_then(|discord| discord.application_id)
            .unwrap_or(String::from("1053747938519679018"))
            .as_str(),
    )
    .expect("Failed to create Discord RPC client, discord is down or the Client ID is invalid.");

    // Start up the client connection, so that we can actually send and receive stuff
    jellyfin_rpc::connect(&mut rich_presence_client);
    info!(
        "{}",
        "Connected to Discord Rich Presence Socket"
            .bright_green()
            .bold(),
    );

    // Start loop
    loop {
        let mut content = Content::try_get(&config, 1).await;

        let mut blacklist_check = true;
        config
            .clone()
            .jellyfin
            .blacklist
            .and_then(|blacklist| blacklist.media_types)
            .unwrap_or(vec![MediaType::None])
            .iter()
            .for_each(|x| {
                if blacklist_check && !content.media_type.is_none() {
                    blacklist_check = content.media_type != *x
                }
            });
        if config
            .clone()
            .jellyfin
            .blacklist
            .and_then(|blacklist| blacklist.libraries)
            .is_some()
        {
            for library in &config
                .clone()
                .jellyfin
                .blacklist
                .and_then(|blacklist| blacklist.libraries)
                .unwrap()
            {
                if blacklist_check && !content.media_type.is_none() {
                    blacklist_check = jellyfin::library_check(
                        &config.jellyfin.url,
                        &config.jellyfin.api_key,
                        &content.item_id,
                        library,
                        config.jellyfin.self_signed_cert.unwrap_or(false),
                    )
                    .await?;
                }
            }
        }

        if !content.media_type.is_none()
            && blacklist_check
            && show_paused(&content.media_type, content.endtime, &config.discord)
        {
            // Print what we're watching
            if !connected {
                info!("{}", content.details.bright_cyan().bold());
                info!("{}", content.state_message.bright_cyan().bold());
                // Set connected to true so that we don't try to connect again
                connected = true;
            }
            if config
                .clone()
                .images
                .and_then(|images| images.imgur_images)
                .unwrap_or(false)
                && content.media_type != MediaType::LiveTv
            {
                content.image_url = Imgur::get(
                    &content.image_url,
                    &content.item_id,
                    &config
                        .clone()
                        .imgur
                        .and_then(|imgur| imgur.client_id)
                        .expect("Imgur client ID cant be loaded."),
                    args.image_urls.clone(),
                    config.jellyfin.self_signed_cert.unwrap_or(false),
                )
                .await
                .unwrap_or_else(|e| {
                    error!("{}", format!("Failed to use Imgur: {:?}", e).red().bold());
                    Imgur::default()
                })
                .url;
            }

            // Set the activity
            let mut rpcbuttons: Vec<activity::Button> = vec![];
            let mut x = 0;
            let buttons = config
                .clone()
                .discord
                .and_then(|discord| discord.buttons)
                .unwrap_or(vec![config::Button::default(), config::Button::default()]);

            // For loop to determine if external services are to be used or if there are custom buttons instead
            for button in buttons.iter() {
                if button.name == "dynamic"
                    && button.url == "dynamic"
                    && content.external_services.len() != x
                {
                    rpcbuttons.push(activity::Button::new(
                        &content.external_services[x].name,
                        &content.external_services[x].url,
                    ));
                    x += 1
                } else if button.name != "dynamic" || button.url != "dynamic" {
                    rpcbuttons.push(activity::Button::new(&button.name, &button.url))
                }

                // Exit early if there's 2 buttons already present, as this is Discord's cap
                if rpcbuttons.len() == 2 {
                    break;
                }
            }

            rich_presence_client
                .set_activity(jellyfin_rpc::setactivity(
                    &content.state_message,
                    &content.details,
                    content.endtime,
                    &content.image_url,
                    rpcbuttons,
                    format!("Jellyfin-RPC v{}", VERSION.unwrap_or("0.0.0")).as_str(),
                    &content.media_type,
                ))
                .unwrap_or_else(|err| {
                    error!("{}\nError: {}", "Failed to set activity".red().bold(), err);
                    retry_with_index(
                        retry::delay::Exponential::from_millis(1000),
                        |current_try| {
                            info!(
                                "{} {}{}",
                                "Attempt".bold().truecolor(225, 69, 0),
                                current_try.to_string().bold().truecolor(225, 69, 0),
                                ": Trying to reconnect".bold().truecolor(225, 69, 0)
                            );
                            match rich_presence_client.reconnect() {
                                Ok(result) => retry::OperationResult::Ok(result),
                                Err(err) => {
                                    error!(
                                        "{}\nError: {}",
                                        "Failed to reconnect, retrying soon".red().bold(),
                                        err
                                    );
                                    retry::OperationResult::Retry(())
                                }
                            }
                        },
                    )
                    .unwrap();
                    info!(
                        "{}",
                        "Reconnected to Discord Rich Presence Socket"
                            .bright_green()
                            .bold(),
                    );
                    info!("{}", content.details.bright_cyan().bold());
                    info!("{}", content.state_message.bright_cyan().bold());
                });
        } else if connected {
            // Disconnect from the client
            rich_presence_client
                .clear_activity()
                .expect("Failed to clear activity");
            // Set connected to false so that we dont try to disconnect again
            connected = false;
            info!("{}", "Cleared Rich Presence".bright_red().bold(),);
        }

        tokio::time::sleep(tokio::time::Duration::from_secs(args.wait_time as u64)).await;
    }
}
