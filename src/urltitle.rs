/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use chrono::prelude::*;
use http::Method;
use log::debug;
use regex::Regex;
use reqwest::header::{HeaderMap, HeaderName, CONTENT_LENGTH, CONTENT_TYPE};
use select::document::Document;
use select::predicate::Name;
use tokio::sync::mpsc;

use crate::botaction::{ActionType, BotAction};
use crate::http_client::HTTP_CLIENT;
use crate::IrcChannel;

lazy_static! {
    static ref RE_URL: Regex = Regex::new(r"(https?://[^ ]+)").unwrap();
}

async fn title_from_url(url: &str) -> Option<String> {
    debug!("Trying to get title for url {}", url);

    lazy_static! {
        static ref RE_TWITTER_URL: Regex =
            Regex::new(r"https?://(?:mobile\.)?twitter\.com/[^/]+/status/(?P<id>\d+)/?.*").unwrap();
        static ref RE_WIKIPEDIA_URL: Regex =
            Regex::new(r"https?://(?P<lang>..)\.wikipedia.org/wiki/(?P<title>[^/]+)").unwrap();
    }

    if RE_TWITTER_URL.is_match(url) {
        let caps = RE_TWITTER_URL.captures(url)?;
        let id = caps.name("id")?.as_str();
        debug!("Looks like a Twitter URL");
        return parse_twitter(id).await;
    }
    if RE_WIKIPEDIA_URL.is_match(url) {
        let caps = RE_WIKIPEDIA_URL.captures(url)?;
        let title = caps.name("title")?.as_str();
        let lang = caps.name("lang")?.as_str();
        debug!("Looks like a Wikipedia URL");
        return parse_wikipedia(lang, title).await;
    }

    let resp = match HTTP_CLIENT.get(url).send().await {
        Ok(r) => r,
        Err(e) => {
            debug!("Could not get url {}: {}", url, e);
            return None;
        }
    };

    let headers = resp.headers();
    if headers.contains_key(CONTENT_TYPE)
        && !headers[CONTENT_TYPE]
            .to_str()
            .unwrap()
            .starts_with("text/html")
    {
        debug!("Not a HTML file");
        return None;
    }

    if headers.contains_key(CONTENT_LENGTH) {
        let len_str = match headers[CONTENT_LENGTH].to_str() {
            Ok(s) => s,
            Err(_) => {
                return None;
            }
        };
        let length = match len_str.parse::<u64>() {
            Ok(l) => l / 1024, // Get the length in kilobytes
            Err(_) => {
                return None;
            }
        };
        if length > 2048 {
            debug!("Content length > 2MB, not fetching");
            return None;
        }
    }

    let body = match resp.text().await {
        Ok(b) => b,
        Err(_) => {
            return None;
        }
    };

    let document = Document::from(body.as_str());
    let mut found_title = None;

    for node in document.find(Name("meta")) {
        if let Some(t) = node.attr("property") {
            if t == "og:title" {
                if let Some(title) = node.attr("content") {
                    debug!("Title found in og:title");
                    found_title = Some(title.to_string());
                }
            }
        }
    }

    if found_title.is_none() {
        if let Some(node) = document.find(Name("title")).next() {
            debug!("Title found in title tag");
            found_title = Some(node.text());
        }
    }

    match found_title {
        Some(mut title) => {
            title = title.replace('\n', " ");
            title = title.replace('\r', " ");
            title = title.replace('\t', " ");
            let trimmed = title.trim();

            Some(format!("Title: {}", trimmed))
        }
        None => None,
    }
}

async fn send_title(sender: mpsc::Sender<BotAction>, target: IrcChannel, url: &str) {
    if let Some(t) = title_from_url(url).await {
        sender
            .send(BotAction {
                target,
                action_type: ActionType::Message(t),
            })
            .await
            .unwrap();
    }
}

pub async fn handle_url_titles(sender: mpsc::Sender<BotAction>, source: IrcChannel, msg: &str) {
    for mat in RE_URL.find_iter(msg) {
        let url = mat.as_str().to_string();
        debug!("URL DETECTED: {}", url);

        let s = sender.clone();
        let src = IrcChannel {
            network: source.network.to_owned(),
            channel: source.channel.to_owned(),
        };
        tokio::spawn(async move {
            send_title(s, src, &url).await;
        });
    }
}

async fn parse_wikipedia(lang: &str, title: &str) -> Option<String> {
    if let Ok(summary) = crate::wikipedia::get_summary(lang, title).await {
        Some(format!("Title: {}", summary))
    } else {
        None
    }
}

pub async fn parse_twitter(id: &str) -> Option<String> {
    async fn get_token(auth: &str) -> Option<String> {
        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static("authorization"),
            auth.parse().unwrap(),
        );
        headers.insert(
            reqwest::header::ACCEPT,
            "text/html,application/xhtml+xml,application/xml;q=0.9,image/webp,*/*;q=0.8"
                .parse()
                .unwrap(),
        );
        headers.insert(
            reqwest::header::ACCEPT_LANGUAGE,
            "en-US,en;q=0.5".parse().unwrap(),
        );
        headers.insert(reqwest::header::CONNECTION, "keep-alive".parse().unwrap());

        let activate_url = "https://api.twitter.com/1.1/guest/activate.json";

        let request = HTTP_CLIENT
            .request(Method::POST, activate_url)
            .headers(headers)
            .send()
            .await;

        let resp = match request {
            Ok(r) => {
                if let Ok(re) = r.text().await {
                    re
                } else {
                    return None;
                }
            }
            Err(_) => {
                return None;
            }
        };
        let json: serde_json::Value = match serde_json::from_str(&resp) {
            Ok(j) => j,
            Err(_) => {
                return None;
            }
        };

        json["guest_token"].as_str().map(|token| token.to_owned())
    }

    fn timestr(time: &str) -> String {
        let dt = match DateTime::parse_from_str(time, "%a %b %d %H:%M:%S %z %Y") {
            Ok(d) => d.naive_utc(),
            Err(_) => {
                return "".to_string();
            }
        };
        let now = Utc::now().naive_utc();
        let diff = now - dt;

        let approx;
        if diff.num_days() >= (30 * 6) {
            approx = dt.format("%Y-%m-%d").to_string();
        } else if diff.num_days() > 30 {
            approx = dt.format("%b %d").to_string();
        } else if diff.num_days() >= 1 {
            approx = format!("{}d", diff.num_days());
        } else if diff.num_hours() >= 1 {
            approx = format!("{}h", diff.num_hours());
        } else {
            approx = format!("{}min", diff.num_minutes());
        }

        approx
    }

    let auth = "Bearer AAAAAAAAAAAAAAAAAAAAANRILgAAAAAAnNwIzUejRCOuH5E6I8xnZz4puTs%3D1Zv7ttfk8LF81IUq16cHjhLTvJu4FA33AGWWjCpTnA";
    let token = get_token(auth).await?;

    let mut headers = HeaderMap::new();
    headers.insert(
        HeaderName::from_static("authorization"),
        auth.parse().unwrap(),
    );
    headers.insert(reqwest::header::CONNECTION, "keep-alive".parse().unwrap());
    headers.insert(
        reqwest::header::CONTENT_TYPE,
        "application/json".parse().unwrap(),
    );
    headers.insert(
        HeaderName::from_static("x-guest-token"),
        token.parse().unwrap(),
    );
    headers.insert(
        HeaderName::from_static("x-twitter-active-user"),
        "yes".parse().unwrap(),
    );
    headers.insert(
        HeaderName::from_static("authority"),
        "api.twitter.com".parse().unwrap(),
    );
    headers.insert(
        reqwest::header::ACCEPT_LANGUAGE,
        "en-US,en;q=0.9".parse().unwrap(),
    );
    headers.insert(reqwest::header::ACCEPT, "*/*".parse().unwrap());
    headers.insert(reqwest::header::DNT, "1".parse().unwrap());

    let request = HTTP_CLIENT
        .request(
            Method::GET,
            &format!(
                "https://api.twitter.com/2/timeline/conversation/{}.json",
                id
            ),
        )
        .headers(headers)
        .query(&[("tweet_mode", "extended")])
        .send()
        .await;

    let resp = match request {
        Ok(r) => {
            if let Ok(re) = r.text().await {
                re
            } else {
                return None;
            }
        }
        Err(_) => {
            return None;
        }
    };
    let json: serde_json::Value = match serde_json::from_str(&resp) {
        Ok(j) => j,
        Err(_) => {
            return None;
        }
    };

    let tweet_info = &json["globalObjects"]["tweets"][id];
    let text = tweet_info["full_text"].as_str()?;
    let retweets = tweet_info["retweet_count"].as_u64()?;
    let favorites = tweet_info["favorite_count"].as_u64()?;
    let created = tweet_info["created_at"].as_str()?;
    let user_id = tweet_info["user_id_str"].as_str()?;

    let user_info = &json["globalObjects"]["users"][user_id];
    let fullname = user_info["name"].as_str()?;
    let screenname = user_info["screen_name"].as_str()?;
    let verified = !matches!(user_info["verified"], serde_json::Value::Null);
    let print_name = match verified {
        true => format!("{} (✔{})", fullname, screenname),
        false => format!("{} ({})", fullname, screenname),
    };

    return Some(format!(
        "Title: {} {}: {} [♻ {} ♥ {}]",
        print_name,
        timestr(created),
        text,
        retweets,
        favorites
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn urltitle_yle() {
        let url = "https://yle.fi/uutiset/3-11499937";
        let expected_title = "Title: Suomalaistutkijat löysivät krapulaa helpottavan aineen – koetilanteessa haasteensa: osa ei pystynyt juomaan riittävästi, osa ei malttanut lopettaa".to_string();
        let title = title_from_url(url).await;

        assert_eq!(title, Some(expected_title));
    }

    #[tokio::test]
    async fn urltitle_wikipedia() {
        let url = "https://en.wikipedia.org/wiki/Koro_(medicine)";
        let expected_title = "Title: Koro is a culture bound delusional disorder in which an individual has an overpowering belief that their sex organs are retracting and will disappear, despite the lack of any true longstanding changes to the genitals.  Koro is also known as shrinking penis, and it is listed in the Diagnostic and Statistical Manual of Mental Disorders. / The syndrome occurs worldwide, and mass hysteria of genital-shrinkage anxiety has a history in Africa, Asia and Europe.".to_string();
        let title = title_from_url(url).await;

        assert_eq!(title, Some(expected_title));
    }

    #[tokio::test]
    async fn urltitle_youtube() {
        let url = "https://www.youtube.com/watch?v=2XLZ4Z8LpEE";
        let expected_title = "Title: Using a 1930 Teletype as a Linux Terminal".to_string();
        let title = title_from_url(url).await;

        assert_eq!(title, Some(expected_title));
    }

    #[tokio::test]
    async fn urltitle_hsfi() {
        let url = "https://www.hs.fi/talous/art-2000007711427.html";
        let expected_title =
            "Title: ATK | Brexit-sopimus kehottaa käyttämään ikivanhaa tekniikkaa kuten Netscape-selainta ja SHA-1-salausta"
                .to_string();
        let title = title_from_url(url).await;

        assert_eq!(title, Some(expected_title));
    }

    #[tokio::test]
    async fn urltitle_twitter() {
        let url = "https://twitter.com/BillGates/status/1352662770416664577";
        let re_expected_title = Regex::new(
            r"^Title: Bill Gates \(✔BillGates\) [^:]*: One of the benefits of being 65 is that I’m eligible for the COVID-19 vaccine. I got my first dose this week, and I feel great. Thank you to all of the scientists, trial participants, regulators, and frontline healthcare workers who got us to this point. https://t.co/67SIfrG1Yd \[♻ \d+ ♥ \d+\]$").unwrap();
        let title = title_from_url(url).await.unwrap();

        assert!(re_expected_title.is_match(&title));
    }
}
