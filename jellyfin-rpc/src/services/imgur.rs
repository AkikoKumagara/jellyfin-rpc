use crate::core::error::ImgurError;
use log::debug;
use serde_json::{json, Value};
use std::io::Write;
use std::{env, fs, path};

/// Struct containing the Imgur URL of the currently playing item.
#[derive(Default)]
pub struct Imgur {
    pub url: String,
}

/// Find urls.json in filesystem, used to store images that were already previously uploaded to imgur.
///
/// This is to avoid the user having to specify a filepath on launch.
///
/// Default urls.json path depends on OS
/// Windows: `%appdata%\jellyfin-rpc\urls.json`
/// Linux/macOS: `~/.config/jellyfin-rpc/urls.json`
pub fn get_urls_path() -> Result<String, ImgurError> {
    if cfg!(not(windows)) {
        debug!("Platform is not Windows");
        let xdg_config_home = match env::var("XDG_CONFIG_HOME") {
            Ok(xdg_config_home) => xdg_config_home,
            Err(_) => env::var("HOME")? + "/.config",
        };

        Ok(xdg_config_home + ("/jellyfin-rpc/urls.json"))
    } else {
        debug!("Platform is Windows");
        let app_data = env::var("APPDATA")?;
        Ok(app_data + r"\jellyfin-rpc\urls.json")
    }
}

impl Imgur {
    /// Queries the urls.json file for an imgur url with the same item ID attached.
    ///
    /// If there's no imgur URL in the file, it will upload the image to imgur, store it in the file and then hand the URL over in a result.
    pub async fn get(
        image_url: &str,
        item_id: &str,
        client_id: &str,
        image_urls_file: Option<String>,
        self_signed_cert: bool,
    ) -> Result<Self, ImgurError> {
        let file = match image_urls_file {
            Some(file) => file,
            None => get_urls_path()?,
        };

        debug!("Imgur urls path is: {}", file);

        let mut json = Imgur::read_file(file.clone())?;
        if let Some(value) = json.get(item_id).and_then(Value::as_str) {
            debug!("Found imgur url {} for item id {}", value, item_id);
            return Ok(Self {
                url: value.to_string(),
            });
        }

        Ok(Self {
            url: Imgur::write_file(
                file,
                image_url,
                item_id,
                client_id,
                self_signed_cert,
                &mut json,
            )
            .await?,
        })
    }

    fn read_file(file: String) -> Result<Value, ImgurError> {
        let content = match fs::read_to_string(file.clone()) {
            Ok(content) => content,
            Err(_) => {
                // Create directories
                let path = path::Path::new(&file).parent().ok_or(ImgurError::from(
                    std::io::Error::new(std::io::ErrorKind::NotFound, file.clone()),
                ))?;
                fs::create_dir_all(path)?;

                // Create urls.json file
                fs::File::create(file.clone()).map(|mut file| {
                    write!(file, "{{\n}}").ok();
                    file
                })?;

                // Read the newly created file
                fs::read_to_string(file.clone())?
            }
        };

        let json: Value = serde_json::from_str(&content)?;
        Ok(json)
    }

    async fn write_file(
        file: String,
        image_url: &str,
        item_id: &str,
        client_id: &str,
        self_signed_cert: bool,
        json: &mut Value,
    ) -> Result<String, ImgurError> {
        // Create a new map that's used for adding data to the "urls.json" file
        let mut new_data = serde_json::Map::new();
        // Upload the content's image to imgur
        let imgur_url = Imgur::upload(image_url, client_id, self_signed_cert).await?;
        // Insert the item_id and the new image url into the map we created earlier
        new_data.insert(item_id.to_string(), json!(imgur_url));

        // Turn the old json data into a map and append the new map to the old one
        let data = json.as_object_mut().ok_or(ImgurError::None)?; //.expect("\"urls.json\" file is not an object, try deleting the file and running the program again.");
        data.append(&mut new_data);

        // Overwrite the "urls.json" file with the new data
        write!(fs::File::create(file)?, "{}", json!(data))?;
        Ok(imgur_url)
    }

    async fn upload(
        image_url: &str,
        client_id: &str,
        self_signed_cert: bool,
    ) -> Result<String, ImgurError> {
        let img = crate::get(image_url, self_signed_cert)
            .await?
            .bytes()
            .await?;

        debug!("Got image bytes from url: {}", image_url);

        let client = reqwest::Client::new();
        let response = client
            .post("https://api.imgur.com/3/image")
            .header(
                reqwest::header::AUTHORIZATION,
                format!("Client-ID {}", client_id),
            )
            .body(img)
            .send()
            .await?;
        let val: Value = response.json().await?;

        debug!("Imgur response after upload: {:#?}", val);

        let image_url = val["data"]["link"]
            .as_str()
            .ok_or(ImgurError::InvalidResponse)?
            .to_string();

        debug!("Image uploaded to {}", image_url);

        Ok(image_url)
    }
}
