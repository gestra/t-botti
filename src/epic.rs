/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use chrono::prelude::*;
use tokio::sync::mpsc;

use crate::botaction::{ActionType, BotAction};
use crate::http_client::HTTP_CLIENT;
use crate::IrcChannel;

async fn get_json() -> reqwest::Result<String> {
    let baseurl = "https://store-site-backend-static.ak.epicgames.com/freeGamesPromotions?locale=en-US&country=FI&allowCountries=FI";

    let json = HTTP_CLIENT.get(baseurl).send().await?.text().await?;

    Ok(json)
}

fn parse_json(json_text: &str) -> Result<Vec<String>, String> {
    let mut free_game_names = Vec::new();

    let json: serde_json::Value = match serde_json::from_str(json_text) {
        Ok(j) => j,
        Err(_) => {
            return Err("Error parsing JSON".to_owned());
        }
    };

    let games = &json["data"]["Catalog"]["searchStore"]["elements"];

    if let Some(gamelist) = games.as_array() {
        for game in gamelist {
            let title = match game["title"].as_str() {
                Some(s) => {
                    if s == "Mystery Game" {
                        continue;
                    }
                    s
                }
                _ => {
                    continue;
                }
            };

            if let Some(price) = game["price"]["totalPrice"]["discountPrice"].as_u64() {
                if price != 0 {
                    continue;
                }
            } else {
                continue;
            }
            let offer = &game["promotions"]["promotionalOffers"][0]["promotionalOffers"][0];

            let find_offer_date_strings = || -> Option<(&str, &str)> {
                let start = offer["startDate"].as_str()?;
                let end = offer["endDate"].as_str()?;
                Some((start, end))
            };
            if let Some((start_str, end_str)) = find_offer_date_strings() {
                let offer_valid = || -> Result<bool, chrono::ParseError> {
                    let start = start_str.parse::<DateTime<Utc>>()?;
                    let end = end_str.parse::<DateTime<Utc>>()?;
                    let now = Utc::now();
                    if start > now || end < now {
                        return Ok(false);
                    }
                    Ok(true)
                };
                match offer_valid() {
                    Ok(true) => {}
                    _ => {
                        continue;
                    }
                }
            } else {
                continue;
            }

            free_game_names.push(title.to_owned());
        }
    } else {
        return Err("No games found".to_owned());
    }

    Ok(free_game_names)
}

fn generate_msg(games: Vec<String>) -> String {
    let msg: String;

    if games.is_empty() {
        msg = "Ei ilmaisia pelejä Epicissä.".to_owned();
    } else {
        msg = format!("Epicissä nyt ilmaiseksi: {}", games.join(", "));
    }

    msg
}

pub async fn command_epic(bot_sender: mpsc::Sender<BotAction>, source: IrcChannel) {
    let msg;
    if let Ok(json) = get_json().await {
        msg = match parse_json(&json) {
            Ok(data) => generate_msg(data),
            Err(_) => "Virhe ilmaispelien haussa".to_owned(),
        };
    } else {
        msg = "Virhe ilmaispelien haussa".to_owned();
    }

    let action = BotAction {
        target: source,
        action_type: ActionType::Message(msg),
    };

    bot_sender.send(action).await.unwrap();
}
