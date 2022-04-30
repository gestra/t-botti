/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use chrono::prelude::*;
use log::{debug, error, warn};
use tokio::sync::mpsc;

use crate::botaction::{ActionType, BotAction};
use crate::http_client::HTTP_CLIENT;
use crate::IrcChannel;

#[derive(Debug)]
enum ShowStatus {
    Running,
    Ended,
    InDevelopment,
    Tbd,
}

#[derive(Debug)]
struct EpData {
    name: Option<String>,
    airdate: Option<DateTime<FixedOffset>>,
    season: Option<i64>,
    number: Option<i64>,
}

#[derive(Debug)]
struct ShowData {
    showname: String,
    status: Option<ShowStatus>,
    previousep: Option<EpData>,
    nextep: Option<EpData>,
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

async fn get_url(url: &str) -> reqwest::Result<String> {
    let j = HTTP_CLIENT.get(url).send().await?.text().await?;
    Ok(j)
}

async fn get_ep_info(url: &str) -> Result<EpData, String> {
    if let Ok(epjson) = get_url(url).await {
        let nextj: serde_json::Value = match serde_json::from_str(&epjson) {
            Ok(j) => j,
            Err(_) => {
                return Err("Error parsing JSON".to_owned());
            }
        };

        let airdate = if let Some(airstamp) = nextj["airstamp"].as_str() {
            if let Ok(dt) = DateTime::parse_from_rfc3339(airstamp) {
                Some(dt)
            } else {
                None
            }
        } else {
            None
        };

        let name = nextj["name"].as_str().map(|n| n.to_owned());

        let season = nextj["season"].as_i64();
        let number = nextj["number"].as_i64();

        return Ok(EpData {
            name,
            airdate,
            season,
            number,
        });
    }

    Err("Error parsing JSON".to_owned())
}

fn last_ep_from_eplist(json: &serde_json::Value) -> Option<EpData> {
    if let Some(eps) = json["_embedded"]["episodes"].as_array() {
        if let Some(ep) = eps.last() {
            let airdate = if let Some(airstamp) = ep["airstamp"].as_str() {
                if let Ok(dt) = DateTime::parse_from_rfc3339(airstamp) {
                    Some(dt)
                } else {
                    None
                }
            } else {
                None
            };

            let name = ep["name"].as_str().map(|n| n.to_owned());

            let season = ep["season"].as_i64();
            let number = ep["number"].as_i64();

            return Some(EpData {
                name,
                airdate,
                season,
                number,
            });
        }
    }

    None
}

fn next_ep_from_eplist(json: &serde_json::Value) -> Option<EpData> {
    let now: DateTime<Utc> = Utc::now();

    let mut airdate = None;
    let mut name = None;
    let mut season = None;
    let mut number = None;

    if let Some(eps) = json["_embedded"]["episodes"].as_array() {
        for ep in eps {
            if let Some(airstamp) = ep["airstamp"].as_str() {
                if let Ok(dt) = DateTime::parse_from_rfc3339(airstamp) {
                    if dt > now {
                        airdate = Some(dt);
                        if let Some(n) = ep["name"].as_str() {
                            name = Some(n.to_owned());
                        }
                        season = ep["season"].as_i64();
                        number = ep["number"].as_i64();
                        break;
                    }
                }
            }
        }

        if name.is_some() && airdate.is_some() && season.is_some() && number.is_some() {
            return Some(EpData {
                name,
                airdate,
                season,
                number,
            });
        }
    }

    None
}

async fn parse_json(json_text: &str) -> Result<ShowData, String> {
    let mut showname = String::new();
    let mut status = None;
    let mut nextep = None;
    let mut previousep = None;

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
        status = match r {
            "Running" => Some(ShowStatus::Running),
            "Ended" => Some(ShowStatus::Ended),
            "In Development" => Some(ShowStatus::InDevelopment),
            "To Be Determined" => Some(ShowStatus::Tbd),
            _ => {
                warn!("Unknown status: {}", r);
                None
            }
        };
    }

    if let Some(nextepurl) = json["_links"]["nextepisode"]["href"].as_str() {
        if let Ok(ep) = get_ep_info(nextepurl).await {
            nextep = Some(ep);
        }
    }

    if let Some(previousepurl) = json["_links"]["previousepisode"]["href"].as_str() {
        if let Ok(ep) = get_ep_info(previousepurl).await {
            previousep = Some(ep);
        }
    }

    match status {
        Some(ShowStatus::Running) => {
            if nextep.is_none() {
                nextep = next_ep_from_eplist(&json);
            }
        }
        Some(ShowStatus::Ended) => {
            if previousep.is_none() {
                previousep = last_ep_from_eplist(&json);
            }
        }
        Some(ShowStatus::InDevelopment) => {
            debug!("Show in development");
            if nextep.is_none() {
                nextep = next_ep_from_eplist(&json);
            }
        }
        Some(ShowStatus::Tbd) => {
            if nextep.is_none() {
                nextep = next_ep_from_eplist(&json);
            }
            if previousep.is_none() {
                previousep = last_ep_from_eplist(&json);
            }
        }
        None => {}
    }

    Ok(ShowData {
        showname,
        status,
        nextep,
        previousep,
    })
}

fn generate_msg(data: ShowData) -> String {
    fn time_from_last_ep(dt: DateTime<FixedOffset>) -> String {
        let today = Local::now().date();
        let dur = dt.date().signed_duration_since(today);
        let days = -dur.num_days();
        match days {
            0 => ", today".to_string(),
            1 => ", yesterday".to_string(),
            2..=364 => format!(", {} days ago", days),
            365..=729 => ", 1 year ago".to_string(),
            730.. => format!(", {} years ago", days / 365),
            _ => {
                error!("Days since episode airing was positive");
                "".to_string()
            }
        }
    }
    fn time_until_next_ep(dt: DateTime<FixedOffset>) -> String {
        let today = Local::now().date();
        let dur = dt.date().signed_duration_since(today);
        let days = dur.num_days();
        match days {
            0 => ", today".to_string(),
            1 => ", tomorrow".to_string(),
            2.. => format!(", {} days from now", days),
            _ => {
                error!("Time until episode airdate was negative");
                "".to_string()
            }
        }
    }

    fn next_ep_msg(data: &ShowData) -> String {
        let msg;
        if let Some(nextep) = &data.nextep {
            if let Some(date) = nextep.airdate {
                let datefmt = format!("{}-{:02}-{:02}", date.year(), date.month(), date.day());
                let from_now = time_until_next_ep(date);

                if nextep.season.is_some() && nextep.number.is_some() && nextep.name.is_some() {
                    msg = format!(
                        "Next episode of {} {}x{} '{}' airs on {}{}",
                        data.showname,
                        nextep.season.unwrap(),
                        nextep.number.unwrap(),
                        nextep.name.as_ref().unwrap(),
                        datefmt,
                        from_now,
                    );
                } else if nextep.name.is_some() {
                    msg = format!(
                        "Next episode of {} '{}' airs on {}{}",
                        data.showname,
                        nextep.name.as_ref().unwrap(),
                        datefmt,
                        from_now,
                    );
                } else {
                    msg = format!("Next episode of {} airs on {}", data.showname, datefmt,);
                }
            } else {
                msg = format!("Next episode of {} not found", data.showname);
            }
        } else if let Some(prevep) = &data.previousep {
            if prevep.number.is_some() && prevep.season.is_some() && prevep.airdate.is_some() {
                let airdate = prevep.airdate.unwrap();
                let datefmt = format!(
                    "{}-{:02}-{:02}",
                    airdate.year(),
                    airdate.month(),
                    airdate.day()
                );
                let from_now = time_from_last_ep(airdate);
                msg = format!(
                        "No airdate found for next episode of {}. Last episode {}x{} '{}' aired on {}{}",
                        data.showname,
                        prevep.season.unwrap(),
                        prevep.number.unwrap(),
                        prevep.name.as_ref().unwrap(),
                        datefmt,
                        from_now,
                    );
            } else {
                msg = format!("No episode of {} found", data.showname);
            }
        } else {
            msg = format!("No airdate found for next episode of {}", data.showname);
        }

        msg
    }

    let msg;

    match data.status {
        Some(ShowStatus::Running) => {
            msg = next_ep_msg(&data);
        }
        Some(ShowStatus::Ended) => {
            if let Some(prevep) = data.previousep {
                if let Some(date) = prevep.airdate {
                    let datefmt = format!("{}-{:02}-{:02}", date.year(), date.month(), date.day());
                    let from_now = time_from_last_ep(date);

                    if prevep.name.is_some() && prevep.number.is_some() && prevep.season.is_some() {
                        let name = prevep.name.unwrap();
                        let epnum = prevep.number.unwrap();
                        let epseason = prevep.season.unwrap();
                        msg = format!(
                            "Last episode of {} {}x{} '{}' aired on {}{}",
                            data.showname, epseason, epnum, name, datefmt, from_now
                        );
                    } else {
                        msg = format!("{} ended on {}{}", data.showname, datefmt, from_now);
                    }
                } else {
                    msg = format!("{} has ended", data.showname);
                }
            } else {
                msg = format!("{} has ended", data.showname);
            }
        }
        Some(ShowStatus::InDevelopment) => {
            if let Some(nextep) = data.nextep {
                if let Some(date) = nextep.airdate {
                    let datefmt = format!("{}-{:02}-{:02}", date.year(), date.month(), date.day());
                    let from_now = time_until_next_ep(date);
                    msg = format!("{} will premiere on {}{}", data.showname, datefmt, from_now);
                } else {
                    msg = format!("{} is in development", data.showname);
                }
            } else {
                msg = format!("{} is in development", data.showname);
            }
        }
        Some(ShowStatus::Tbd) => {
            msg = next_ep_msg(&data);
        }
        None => {
            msg = "Unknown status".to_owned();
        }
    }

    msg
}

pub async fn command_ep(bot_sender: mpsc::Sender<BotAction>, source: IrcChannel, params: &str) {
    let msg = if let Ok(json) = get_json(params).await {
        match parse_json(&json).await {
            Ok(data) => generate_msg(data),
            Err(e) => e,
        }
    } else {
        "TVmaze API error".to_owned()
    };

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
        let data = parse_json(&json).await.unwrap();
        let msg = generate_msg(data);

        let re_episode_found = Regex::new(r"Last episode of Star Trek: The Next Generation 7x26 'All Good Things... \(2\)' aired on 1994-05-23, .* years ago").unwrap();
        assert!(re_episode_found.is_match(&msg));
    }

    #[tokio::test]
    async fn running_series() {
        let json = get_json(&"The Simpsons").await.unwrap();
        let data = parse_json(&json).await.unwrap();
        let msg = generate_msg(data);

        let re_episode_found = Regex::new(r"Next episode of The Simpsons .*airs on.*").unwrap();

        assert!(
            re_episode_found.is_match(&msg)
                || msg == "No airdate found for next episode of The Simpsons"
        );
    }
}
