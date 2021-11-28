/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use log::error;
use tokio::sync::mpsc;

use crate::botaction::{ActionType, BotAction};
use crate::http_client::HTTP_CLIENT;
use crate::IrcChannel;

async fn get_json(title: &str, lang: &str) -> reqwest::Result<String> {
    let baseurl = format!("https://{}.wikipedia.org/w/api.php", lang);

    let json = HTTP_CLIENT
        .get(baseurl)
        .query(&[
            ("action", "query"),
            ("list", "search"),
            ("srlimit", "1"),
            ("srsearch", title),
            ("srinfo", "suggestion"),
            ("format", "json"),
        ])
        .send()
        .await?
        .text()
        .await?;

    Ok(json)
}

fn get_page_title_from_json(json_text: &str) -> Result<String, String> {
    let json: serde_json::Value = match serde_json::from_str(json_text) {
        Ok(j) => j,
        Err(_) => {
            error!("Error parsing JSON");
            return Err("Error parsing title JSON".to_owned());
        }
    };

    if json == serde_json::Value::Null {
        error!("Nothing found");
        return Err("Nothing found".to_owned());
    }

    if let Some(n) = json["query"]["search"][0]["title"].as_str() {
        Ok(n.to_owned())
    } else {
        error!("No title found");
        Err("No title found".to_owned())
    }
}

async fn get_summary_json(title: &str, lang: &str) -> reqwest::Result<String> {
    let baseurl = format!("https://{}.wikipedia.org/w/api.php", lang);
    let json = HTTP_CLIENT
        .get(baseurl)
        .query(&[
            ("action", "query"),
            ("prop", "extracts"),
            ("exsentences", "3"),
            ("exlimit", "1"),
            ("titles", title),
            ("explaintext", "1"),
            ("formatversion", "2"),
            ("format", "json"),
        ])
        .send()
        .await?
        .text()
        .await?;

    Ok(json)
}

pub async fn get_summary(lang: &str, title: &str) -> Result<String, String> {
    if let Ok(json_text) = get_summary_json(title, lang).await {
        let json: serde_json::Value = match serde_json::from_str(&json_text) {
            Ok(j) => j,
            Err(_) => {
                error!("Error parsing summary JSON");
                return Err("Error parsing JSON".to_owned());
            }
        };

        if let Some(e) = json["query"]["pages"][0]["extract"].as_str() {
            let summary = e.replace("\n", " / ");
            return Ok(summary);
        }
    }

    error!("Error parsing summary JSON");

    Err("Error parsing summary JSON".to_owned())
}

async fn wikipedia_summary(
    bot_sender: mpsc::Sender<BotAction>,
    source: IrcChannel,
    title: &str,
    lang: &str,
) {
    let msg;
    if let Ok(json) = get_json(title, lang).await {
        if let Ok(article_title) = get_page_title_from_json(&json) {
            if let Ok(summary) = get_summary(lang, &article_title).await {
                msg = summary;
            } else {
                msg = "API error".to_owned();
            }
        } else {
            msg = "API error".to_owned();
        }
    } else {
        msg = "Wikipedia API error".to_owned();
    }

    let action = BotAction {
        target: source,
        action_type: ActionType::Message(msg),
    };

    bot_sender.send(action).await.unwrap();
}

pub async fn command_wikipedia(
    bot_sender: mpsc::Sender<BotAction>,
    source: IrcChannel,
    params: &str,
) {
    wikipedia_summary(bot_sender, source, params, "en").await;
}

pub async fn command_wikipediafi(
    bot_sender: mpsc::Sender<BotAction>,
    source: IrcChannel,
    params: &str,
) {
    wikipedia_summary(bot_sender, source, params, "fi").await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn en_wikipedia_title() {
        let summary = get_summary(&"en", &"Taiko").await.unwrap();

        assert_eq!(summary, "Taiko (太鼓) are a broad range of Japanese percussion instruments. In Japanese, the term refers to any kind of drum, but outside Japan, it is used specifically to refer to any of the various Japanese drums called wadaiko (和太鼓, \"Japanese drums\") and to the form of ensemble taiko drumming more specifically called kumi-daiko (組太鼓, \"set of drums\"). The process of constructing taiko varies between manufacturers, and the preparation of both the drum body and skin can take several years depending on the method.");
    }
}
