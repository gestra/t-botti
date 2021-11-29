/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use tokio::sync::mpsc;

use crate::botaction::{ActionType, BotAction};
use crate::http_client::HTTP_CLIENT;
use crate::IrcChannel;

async fn get_json(place: &str) -> reqwest::Result<String> {
    let baseurl = "https://nominatim.openstreetmap.org/search";

    let json = HTTP_CLIENT
        .get(baseurl)
        .query(&[("q", place), ("format", "jsonv2")])
        .send()
        .await?
        .text()
        .await?;

    Ok(json)
}

async fn coordinates(place: &str) -> Result<String, ()> {
    let json_text = match get_json(place).await {
        Ok(s) => s,
        Err(_) => {
            return Err(());
        }
    };

    let json: serde_json::Value = match serde_json::from_str(&json_text) {
        Ok(j) => j,
        Err(_) => {
            return Err(());
        }
    };

    if let Some(lat) = json[0]["lat"].as_str() {
        if let Some(lon) = json[0]["lon"].as_str() {
            return Ok(format!("10/{}/{}", lat, lon));
        }
    }

    Err(())
}

pub async fn command_ukkostutka(
    bot_sender: mpsc::Sender<BotAction>,
    source: IrcChannel,
    params: &str,
) {
    let mut coords = "5.47/62.79/25.728".to_owned();

    if !params.is_empty() {
        if let Ok(c) = coordinates(params).await {
            coords = c;
        }
    }

    let msg = format!("https://map.blitzortung.org/#{}", coords);

    let action = BotAction {
        target: source,
        action_type: ActionType::Message(msg),
    };

    bot_sender.send(action).await.unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn hervanta_coords() {
        let r = coordinates(&"Hervanta").await.unwrap();
        assert_eq!(r, "10/61.4509034/23.8514239");
    }
}
