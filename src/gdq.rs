/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use chrono::prelude::*;
use select::document::Document;
use select::predicate::{Predicate, Attr, Class, Name};
use tokio::sync::mpsc;

use crate::botaction::{ActionType, BotAction};
use crate::http_client::HTTP_CLIENT;
use crate::IrcChannel;

async fn get_html() -> reqwest::Result<String> {
    let baseurl = "https://gamesdonequick.com/schedule";

    let html = HTTP_CLIENT.get(baseurl).send().await?.text().await?;

    Ok(html)
}

fn parse_html(raw_html: &str) -> Result<(String, String), String> {
    let now = Utc::now();
    let mut current = String::new();
    let mut next = String::new();

    let doc = Document::from(raw_html);
    for line in doc.find(Attr("id", "runTable").descendant(Name("tr"))) {
        if let Some(start_time) = line.find(Class("start-time")).next() {
            if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(&start_time.text()) {
                let game_name = line.find(Name("td")).take(2).nth(1).unwrap().text();
                if dt < now {
                    current = game_name;
                } else {
                    next = game_name;
                    break;
                }
            }
        }
    }

    Ok((current, next))
}

fn generate_msg(games: (String, String)) -> String {
    format!("Now playing: {} | Up next: {}", games.0, games.1)
}

pub async fn command_gdq(bot_sender: mpsc::Sender<BotAction>, source: IrcChannel) {
    let html = get_html().await.unwrap();
    let parsed = parse_html(&html).unwrap();
    let msg = generate_msg(parsed);

    let action = BotAction {
        target: source,
        action_type: ActionType::Message(msg),
    };

    bot_sender.send(action).await.unwrap();
}
