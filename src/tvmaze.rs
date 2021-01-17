/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use chrono::prelude::*;
use tokio::sync::mpsc;

use crate::botaction::{ActionType, BotAction};
use crate::http_client::HTTP_CLIENT;
use crate::IrcChannel;

#[derive(Debug)]
struct ShowData {
    showname: String,
    epname: Option<String>,
    epairdate: Option<DateTime<FixedOffset>>,
    epseason: Option<i64>,
    epnumber: Option<i64>,
    running: bool,
}

async fn get_json(showname: &str) -> reqwest::Result<String> {
    let baseurl = "https://api.tvmaze.com/singlesearch/shows";

    let json = HTTP_CLIENT
        .get(baseurl)
        .query(&[("q", showname), ("embed", "episodes")])
        .send()
        .await?
        .text()
        .await?;

    Ok(json)
}

fn parse_json(json_text: &str) -> Result<ShowData, String> {
    let mut showname = String::new();
    let mut epname = None;
    let mut epairdate = None;
    let mut epseason = None;
    let mut epnumber = None;
    let mut running = false;

    let json: serde_json::Value = match serde_json::from_str(json_text) {
        Ok(j) => j,
        Err(_) => {
            return Err("Error parsing JSON".to_owned());
        }
    };

    if json == serde_json::Value::Null {
        return Err("Show not found".to_owned());
    }

    if let Some(n) = json["name"].as_str() {
        showname = n.to_owned();
    }

    if let Some(r) = json["status"].as_str() {
        if r == "Running" {
            running = true;
        }
    }

    if let Some(eps) = json["_embedded"]["episodes"].as_array() {
        if !running {
            if let Some(lastep) = eps.last() {
                if let Some(airstamp) = lastep["airstamp"].as_str() {
                    if let Ok(dt) = DateTime::parse_from_rfc3339(airstamp) {
                        epairdate = Some(dt);
                    }
                }
                if let Some(name) = lastep["name"].as_str() {
                    epname = Some(name.to_owned());
                }
                epseason = lastep["season"].as_i64();
                epnumber = lastep["number"].as_i64();
            }
        } else {
            let now: DateTime<Utc> = Utc::now();
            for ep in eps {
                if let Some(airstamp) = ep["airstamp"].as_str() {
                    if let Ok(dt) = DateTime::parse_from_rfc3339(airstamp) {
                        if dt > now {
                            epairdate = Some(dt);
                            if let Some(name) = ep["name"].as_str() {
                                epname = Some(name.to_owned());
                            }
                            epseason = ep["season"].as_i64();
                            epnumber = ep["number"].as_i64();
                            break;
                        }
                    }
                }
            }
        }
    }

    Ok(ShowData {
        showname,
        epname,
        epairdate,
        epseason,
        epnumber,
        running,
    })
}

fn generate_msg(data: ShowData) -> String {
    let msg;

    if data.running {
        if data.epairdate.is_some() {
            let date = data.epairdate.unwrap();
            let datefmt = format!("{}-{}-{}", date.year(), date.month(), date.day());
            if data.epseason.is_some() && data.epnumber.is_some() && data.epname.is_some() {
                msg = format!(
                    "Next episode of {} {}x{} '{}' airs on {}",
                    data.showname,
                    data.epseason.unwrap(),
                    data.epnumber.unwrap(),
                    data.epname.unwrap(),
                    datefmt,
                );
            } else {
                msg = format!("Next episode of {} airs on {}", data.showname, datefmt,);
            }
        } else {
            msg = format!("No airdate found for next episode of {}", data.showname);
        }
    } else if data.epairdate.is_some() {
        let date = data.epairdate.unwrap();
        let datefmt = format!("{}-{:02}-{:02}", date.year(), date.month(), date.day());

        if data.epname.is_some() && data.epnumber.is_some() && data.epseason.is_some() {
            let name = data.epname.unwrap();
            let epnum = data.epnumber.unwrap();
            let epseason = data.epseason.unwrap();
            msg = format!(
                "Last episode of {} {}x{} '{}' aired on {}",
                data.showname, epseason, epnum, name, datefmt
            );
        } else {
            msg = format!("{} ended on {}", data.showname, datefmt);
        }
    } else {
        msg = format!("{} has ended", data.showname);
    }

    msg
}

pub async fn command_ep(bot_sender: mpsc::Sender<BotAction>, source: IrcChannel, params: &str) {
    let msg;
    if let Ok(json) = get_json(params).await {
        msg = match parse_json(&json) {
            Ok(data) => generate_msg(data),
            Err(e) => e,
        };
    } else {
        msg = "TVmaze API error".to_owned();
    }

    let action = BotAction {
        target: source,
        action_type: ActionType::Message(msg),
    };

    bot_sender.send(action).await.unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;
    use regex::Regex;

    #[tokio::test]
    async fn ended_series() {
        let json = get_json(&"Star Trek The Next Generation").await.unwrap();
        let data = parse_json(&json).unwrap();
        let msg = generate_msg(data);

        assert_eq!(msg, "Last episode of Star Trek: The Next Generation 7x26 'All Good Things... (2)' aired on 1994-05-23");
    }

    #[tokio::test]
    async fn running_series() {
        let json = get_json(&"The Simpsons").await.unwrap();
        let data = parse_json(&json).unwrap();
        let msg = generate_msg(data);

        let re_episode_found = Regex::new("Next episode of The Simpsons .*airs on.*").unwrap();

        assert!(
            re_episode_found.is_match(&msg)
                || msg == "No airdate found for next episode of The Simpsons"
        );
    }
}
