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
struct ShowData {
    showname: String,
    epname: Option<String>,
    lastepname: Option<String>,
    epairdate: Option<DateTime<FixedOffset>>,
    lastepairdate: Option<DateTime<FixedOffset>>,
    epseason: Option<i64>,
    epnumber: Option<i64>,
    lastepseason: Option<i64>,
    lastepnumber: Option<i64>,
    status: Option<ShowStatus>,
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
    let mut lastepname = None;
    let mut epairdate = None;
    let mut lastepairdate = None;
    let mut epseason = None;
    let mut epnumber = None;
    let mut lastepseason = None;
    let mut lastepnumber = None;
    let mut status = None;

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

    let now: DateTime<Utc> = Utc::now();

    match status {
        Some(ShowStatus::Running) => {
            if let Some(eps) = json["_embedded"]["episodes"].as_array() {
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
                if epairdate.is_none() {
                    if let Some(lastep) = eps.last() {
                        if let Some(airstamp) = lastep["airstamp"].as_str() {
                            if let Ok(dt) = DateTime::parse_from_rfc3339(airstamp) {
                                lastepairdate = Some(dt);
                                if let Some(name) = lastep["name"].as_str() {
                                    lastepname = Some(name.to_owned());
                                }
                                lastepseason = lastep["season"].as_i64();
                                lastepnumber = lastep["number"].as_i64();
                            }
                        }
                    }
                }
            }
        }
        Some(ShowStatus::Ended) => {
            if let Some(eps) = json["_embedded"]["episodes"].as_array() {
                if let Some(lastep) = eps.last() {
                    if let Some(airstamp) = lastep["airstamp"].as_str() {
                        if let Ok(dt) = DateTime::parse_from_rfc3339(airstamp) {
                            lastepairdate = Some(dt);
                        }
                    }
                    if let Some(name) = lastep["name"].as_str() {
                        lastepname = Some(name.to_owned());
                    }
                    lastepseason = lastep["season"].as_i64();
                    lastepnumber = lastep["number"].as_i64();
                }
            }
        }
        Some(ShowStatus::InDevelopment) => {
            debug!("Show in development");
            if let Some(eps) = json["_embedded"]["episodes"].as_array() {
                if let Some(firstep) = eps.first() {
                    if let Some(airstamp) = firstep["airstamp"].as_str() {
                        if let Ok(dt) = DateTime::parse_from_rfc3339(airstamp) {
                            epairdate = Some(dt);
                            debug!("Airdate: {:?}", epairdate);
                        }
                    }
                    if let Some(name) = firstep["name"].as_str() {
                        epname = Some(name.to_owned());
                        debug!("Episode name: {:?}", epname);
                    }
                    epseason = firstep["season"].as_i64();
                    debug!("Episode season: {:?}", epseason);
                    epnumber = firstep["number"].as_i64();
                    debug!("Episode number: {:?}", epnumber);
                }
            }
        }
        Some(ShowStatus::Tbd) => {
            if let Some(eps) = json["_embedded"]["episodes"].as_array() {
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
            }
        }
        None => {}
    }

    Ok(ShowData {
        showname,
        epname,
        lastepname,
        epairdate,
        lastepairdate,
        epseason,
        epnumber,
        lastepseason,
        lastepnumber,
        status,
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

    let msg;

    match data.status {
        Some(ShowStatus::Running) => {
            if data.epairdate.is_some() {
                let date = data.epairdate.unwrap();
                let datefmt = format!("{}-{:02}-{:02}", date.year(), date.month(), date.day());
                let from_now = time_until_next_ep(date);

                if data.epseason.is_some() && data.epnumber.is_some() && data.epname.is_some() {
                    msg = format!(
                        "Next episode of {} {}x{} '{}' airs on {}{}",
                        data.showname,
                        data.epseason.unwrap(),
                        data.epnumber.unwrap(),
                        data.epname.unwrap(),
                        datefmt,
                        from_now,
                    );
                } else {
                    msg = format!("Next episode of {} airs on {}", data.showname, datefmt,);
                }
            } else if data.lastepname.is_some()
                && data.lastepnumber.is_some()
                && data.lastepseason.is_some()
                && data.lastepairdate.is_some()
            {
                let airdate = data.lastepairdate.unwrap();
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
                    data.lastepseason.unwrap(),
                    data.lastepnumber.unwrap(),
                    data.lastepname.unwrap(),
                    datefmt,
                    from_now,
                );
            } else {
                msg = format!("No airdate found for next episode of {}", data.showname);
            }
        }
        Some(ShowStatus::Ended) => {
            if let Some(date) = data.lastepairdate {
                let datefmt = format!("{}-{:02}-{:02}", date.year(), date.month(), date.day());
                let from_now = time_from_last_ep(date);

                if data.lastepname.is_some()
                    && data.lastepnumber.is_some()
                    && data.lastepseason.is_some()
                {
                    let name = data.lastepname.unwrap();
                    let epnum = data.lastepnumber.unwrap();
                    let epseason = data.lastepseason.unwrap();
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
        }
        Some(ShowStatus::InDevelopment) => {
            if data.epairdate.is_some() {
                let date = data.epairdate.unwrap();
                let datefmt = format!("{}-{:02}-{:02}", date.year(), date.month(), date.day());
                let from_now = time_until_next_ep(date);
                msg = format!("{} will premiere on {}{}", data.showname, datefmt, from_now);
            } else {
                msg = format!("{} is in development", data.showname);
            }
        }
        Some(ShowStatus::Tbd) => {
            if let Some(date) = data.epairdate {
                let datefmt = format!("{}-{:02}-{:02}", date.year(), date.month(), date.day());
                let from_now = time_from_last_ep(date);
                if data.epname.is_some() && data.epnumber.is_some() && data.epseason.is_some() {
                    msg = format!(
                        "Status of {} is unknown. Last episode {}x{} '{}' aired on {}{}",
                        data.showname,
                        data.epseason.unwrap(),
                        data.epnumber.unwrap(),
                        data.epname.unwrap(),
                        datefmt,
                        from_now,
                    );
                } else {
                    msg = format!(
                        "Status of {} is unknown. Last episode aired on {}{}",
                        data.showname, datefmt, from_now
                    );
                }
            } else {
                msg = format!("Status of {} is unknown", data.showname);
            }
        }
        None => {
            msg = format!("Status of {} is unknown", data.showname);
        }
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
