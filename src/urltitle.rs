/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use log::debug;
use regex::Regex;
use reqwest::header::{CONTENT_LENGTH, CONTENT_TYPE};
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
        static ref RE_WIKIPEDIA_URL: Regex =
            Regex::new(r"https?://(?P<lang>..)\.wikipedia.org/wiki/(?P<title>[^/]+)").unwrap();
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
        let title = title_from_url(url).await;
        assert!(title.unwrap().starts_with("Title: Koro is"));
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
}
