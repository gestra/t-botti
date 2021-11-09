/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use core::time::Duration;

use feed_rs::parser;

use log::{debug, info, warn};

use rusqlite::{named_params, params};

use tokio::sync::mpsc;
use tokio::time::sleep;

use url::Url;

use crate::botaction::{ActionType, BotAction};
use crate::http_client::get_url;
use crate::IrcChannel;

#[derive(Debug)]
pub enum RssCommand {
    Add(String),
    Remove(i64),
    List,
}

#[derive(Debug)]
struct FeedData {
    title: String,
    url: String,
    entries: Vec<feed_rs::model::Entry>,
}

#[derive(Debug)]
pub struct FeedInfo {
    id: i64,
    title: String,
    url: String,
    target: IrcChannel,
}

pub async fn command_rss(sender: mpsc::Sender<BotAction>, source: IrcChannel, params: &str) {
    match rsscommand_from_params(params) {
        Some(RssCommand::Add(url)) => {
            info!(
                "Adding feed to channel {}/{}: {}",
                source.network, source.channel, url
            );
            add_feed(sender, &source, &url).await;
        }
        Some(RssCommand::Remove(id)) => {
            let conn = open_db(false).unwrap();
            let res = remove_feed(&conn, &source, id);
            match res {
                Ok(()) => {
                    info!(
                        "Removed feed id {} from {}/{}",
                        id, source.network, source.channel
                    );
                    sender
                        .send(BotAction {
                            target: source,
                            action_type: ActionType::Message(format!("Removed feed id {}", id)),
                        })
                        .await
                        .unwrap();
                }
                Err(e) => {
                    warn!("Error when removing feed: {}", e);
                    sender
                        .send(BotAction {
                            target: source,
                            action_type: ActionType::Message(e),
                        })
                        .await
                        .unwrap();
                }
            }
        }
        Some(RssCommand::List) => {
            let conn = open_db(false).unwrap();
            let feeds = get_feeds_for_channel(&conn, &source).unwrap();
            list_feeds(sender, &source, feeds).await;
        }
        None => {}
    };
}

fn rsscommand_from_params(s: &str) -> Option<RssCommand> {
    if let Some(params) = s.strip_prefix("add ") {
        let mut iter = params.split_whitespace();
        if let Some(url) = iter.next() {
            if iter.next().is_none() {
                if let Ok(parsed) = Url::parse(url) {
                    if parsed.scheme().starts_with("http") {
                        return Some(RssCommand::Add(url.to_owned()));
                    }
                }
            }
        }
        return None;
    } else if let Some(params) = s.strip_prefix("remove ") {
        if let Ok(id) = params.parse::<i64>() {
            return Some(RssCommand::Remove(id));
        }
        return None;
    } else if s == "list" {
        return Some(RssCommand::List);
    }

    None
}

fn open_db(testing: bool) -> rusqlite::Result<rusqlite::Connection> {
    let conn = match testing {
        true => rusqlite::Connection::open(":memory:")?,
        false => rusqlite::Connection::open("db/rss.db")?,
    };

    conn.execute(
        "create table if not exists feeds (
            id integer primary key,
            url text not null,
            name text not null,
            network text not null,
            channel text not null
        )",
        [],
    )?;
    conn.execute(
        "create table if not exists posts (
            id text PRIMARY KEY,
            url text not null unique,
            title text not null,
            feed references feeds(id)
        )",
        [],
    )?;

    Ok(conn)
}

fn parse_feed(feed: &str, url: &str) -> parser::ParseFeedResult<FeedData> {
    let feed = parser::parse(feed.as_bytes())?;
    let title = match feed.title {
        Some(t) => t.content,
        None => "NoTitle".to_owned(),
    };

    debug!("Parsed feed {}", url);
    debug!("Entries: {:?}", feed.entries);

    Ok(FeedData {
        title,
        url: url.to_owned(),
        entries: feed.entries,
    })
}

async fn add_feed(sender: mpsc::Sender<BotAction>, target: &IrcChannel, url: &str) {
    let feed_body = match get_url(url).await {
        Ok(r) => r,
        Err(_) => {
            warn!("Could not fetch url: {}", url);
            sender
                .send(BotAction {
                    target: IrcChannel {
                        network: target.network.to_owned(),
                        channel: target.channel.to_owned(),
                    },
                    action_type: ActionType::Message(format!(
                        "Error adding feed: Unable to get URL {}",
                        url
                    )),
                })
                .await
                .unwrap();
            return;
        }
    };

    let parsed = match parse_feed(&feed_body, url) {
        Ok(p) => p,
        Err(e) => {
            warn!("Could not parse feed: {:?}", e);
            sender
                .send(BotAction {
                    target: IrcChannel {
                        network: target.network.to_owned(),
                        channel: target.channel.to_owned(),
                    },
                    action_type: ActionType::Message(
                        "Error adding feed: Unable to parse feed.".to_owned(),
                    ),
                })
                .await
                .unwrap();
            return;
        }
    };

    let title = parsed.title.to_owned();

    let conn = open_db(false).unwrap();
    let result = add_feed_to_db(&conn, parsed, target);
    match result {
        Ok(_) => {
            info!("Successfully added feed {}", url);
            sender
                .send(BotAction {
                    target: IrcChannel {
                        network: target.network.to_owned(),
                        channel: target.channel.to_owned(),
                    },
                    action_type: ActionType::Message(format!("Successfully added feed {}", title)),
                })
                .await
                .unwrap();
        }
        Err(e) => {
            warn!("Database error when adding feed: {:?}", e);
            sender
                .send(BotAction {
                    target: IrcChannel {
                        network: target.network.to_owned(),
                        channel: target.channel.to_owned(),
                    },
                    action_type: ActionType::Message(format!(
                        "Error adding feed {}: Database error",
                        title
                    )),
                })
                .await
                .unwrap();
        }
    }
}

fn remove_feed(conn: &rusqlite::Connection, source: &IrcChannel, id: i64) -> Result<(), String> {
    let mut check_feed_stmt = conn
        .prepare(
            "SELECT * FROM feeds WHERE
         id = ?1 AND
         network = ?2 AND
         channel = ?3",
        )
        .unwrap();
    match check_feed_stmt.exists(params![&id, &source.network, &source.channel]) {
        Ok(true) => {}
        Ok(false) => {
            return Err(format!("Feed {} does not exists in this channel", id));
        }
        Err(_) => {
            return Err("Database error".to_owned());
        }
    }

    let mut feed_stmt = conn
        .prepare(
            "DELETE FROM feeds WHERE
         id = :id AND
         network = :network AND
         channel = :channel",
        )
        .unwrap();
    let mut post_stmt = conn
        .prepare(
            "DELETE FROM posts WHERE
         feed = :id",
        )
        .unwrap();

    let feed_exec = feed_stmt.execute(named_params! {
        ":id": &id,
        ":network": &source.network,
        ":channel": &source.channel,
    });
    let post_exec = post_stmt.execute(&[(":id", &id)]);

    if feed_exec.is_err() || post_exec.is_err() {
        return Err("Database error".to_owned());
    }

    Ok(())
}

fn add_feed_to_db(
    conn: &rusqlite::Connection,
    feed_data: FeedData,
    target: &IrcChannel,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO feeds (url, name, network, channel) VALUES (?1, ?2, ?3, ?4)",
        params![
            feed_data.url,
            feed_data.title,
            target.network,
            target.channel
        ],
    )?;

    let feed_id: i64 = conn.query_row(
        "SELECT id FROM feeds WHERE
        url = :url AND
        network = :network AND
        channel = :channel",
        &[
            (":url", &feed_data.url),
            (":network", &target.network),
            (":channel", &target.channel),
        ],
        |row| row.get(0),
    )?;

    // Add all existing entries so we don't flood the channel
    for entry in feed_data.entries {
        if entry.links.is_empty() {
            continue;
        }
        let entry_title = match entry.title {
            Some(t) => t.content,
            None => "".to_string(),
        };
        conn.execute(
            "INSERT INTO posts (id, url, title, feed) VALUES (?1, ?2, ?3, ?4)",
            params![entry.id, entry.links[0].href, entry_title, feed_id],
        )?;
    }

    Ok(())
}

async fn list_feeds(sender: mpsc::Sender<BotAction>, source: &IrcChannel, feeds: Vec<FeedInfo>) {
    for feed in feeds {
        let source_copy = IrcChannel {
            network: source.network.to_owned(),
            channel: source.channel.to_owned(),
        };
        let msg = format!("{}: {} | {}", feed.id, feed.title, feed.url);
        sender
            .send(BotAction {
                target: source_copy,
                action_type: ActionType::Message(msg),
            })
            .await
            .unwrap();
    }
}

fn get_feeds_for_channel(
    conn: &rusqlite::Connection,
    target: &IrcChannel,
) -> rusqlite::Result<Vec<FeedInfo>> {
    let mut feeds = vec![];
    let mut stmt = conn.prepare(
        "SELECT * FROM feeds WHERE
         network = :network AND
         channel = :channel",
    )?;
    let mut rows = stmt.query(&[(":network", &target.network), (":channel", &target.channel)])?;
    while let Some(row) = rows.next()? {
        let id = row.get(0)?;
        let url = row.get(1)?;
        let title = row.get(2)?;

        feeds.push(FeedInfo {
            id,
            url,
            title,
            target: IrcChannel {
                network: target.network.to_owned(),
                channel: target.channel.to_owned(),
            },
        });
    }

    Ok(feeds)
}

fn get_all_feeds(conn: &rusqlite::Connection) -> rusqlite::Result<Vec<FeedInfo>> {
    let mut feeds = vec![];
    let mut stmt = conn.prepare("SELECT * FROM feeds")?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let id = row.get(0)?;
        let url = row.get(1)?;
        let title = row.get(2)?;
        let network = row.get(3)?;
        let channel = row.get(4)?;

        feeds.push(FeedInfo {
            id,
            url,
            title,
            target: IrcChannel { network, channel },
        });
    }

    Ok(feeds)
}

fn entry_is_posted(
    conn: &rusqlite::Connection,
    entry: &feed_rs::model::Entry,
    feed_id: i64,
) -> bool {
    let mut stmt = conn
        .prepare(
            "SELECT * FROM posts WHERE 
            id = ?1 AND
            feed = ?2",
        )
        .unwrap();

    stmt.exists(params![&entry.id, feed_id]).unwrap()
}

fn add_entry_to_db(conn: &rusqlite::Connection, entry: &feed_rs::model::Entry, feed_id: i64) {
    let entry_title = match entry.title {
        Some(ref t) => t.content.to_owned(),
        None => "".to_string(),
    };
    conn.execute(
        "INSERT INTO posts (id, url, title, feed) VALUES (?1, ?2, ?3, ?4)",
        params![entry.id, entry.links[0].href, entry_title, feed_id],
    )
    .unwrap();
}

async fn refresh_feeds(sender: mpsc::Sender<BotAction>) {
    info!("Starting feed refresh");
    let conn = open_db(false).unwrap();
    let feeds = get_all_feeds(&conn).unwrap();
    for feed in feeds {
        let feed_body = match get_url(&feed.url).await {
            Ok(b) => b,
            _ => {
                return;
            }
        };
        let parsed = match parse_feed(&feed_body, &feed.url) {
            Ok(p) => p,
            _ => {
                return;
            }
        };
        let mut to_output = vec![];

        for entry in parsed.entries {
            if !entry.links.is_empty() && !entry_is_posted(&conn, &entry, feed.id) {
                to_output.push(entry);
            }
        }

        for entry in to_output {
            info!(
                "New feed item from feed {} for {}/{}: {}",
                feed.title, feed.target.network, feed.target.channel, feed.title
            );
            let title = match entry.title {
                Some(ref t) => t.content.to_owned(),
                _ => "".to_owned(),
            };
            let output_target = IrcChannel {
                network: feed.target.network.to_owned(),
                channel: feed.target.channel.to_owned(),
            };
            debug!("Entry URL before format!: {}", entry.links[0].href);

            let msg = format!("[{}] {} <{}>", feed.title, title, entry.links[0].href);
            let _ = sender
                .send(BotAction {
                    target: output_target,
                    action_type: ActionType::Message(msg),
                })
                .await;

            add_entry_to_db(&conn, &entry, feed.id);
        }
    }

    info!("Feed refresh finished");
}

pub async fn rss_manager(sender: mpsc::Sender<BotAction>) {
    let update_interval = Duration::from_secs(10 * 60);

    loop {
        tokio::select! {
            _ = sleep(update_interval) => {
                let sender_copy = sender.clone();
                refresh_feeds(sender_copy).await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct FeedEntry {
        feed_id: i64,
        url: String,
        title: String,
    }

    fn get_entries(conn: &rusqlite::Connection, feed_id: i64) -> rusqlite::Result<Vec<FeedEntry>> {
        let mut entries = vec![];
        let mut stmt = conn.prepare(
            "SELECT * FROM posts WHERE
             feed = :feed_id",
        )?;

        let mut rows = stmt.query(&[(":feed_id", &feed_id)])?;
        while let Some(row) = rows.next()? {
            let url = row.get(1)?;
            let title = row.get(2)?;

            entries.push(FeedEntry {
                feed_id,
                url,
                title,
            });
        }

        Ok(entries)
    }

    #[test]
    fn rss_command_parsing_add() {
        let u1 = "http://example.com/feed".to_string();
        let s1 = format!("add {}", u1);
        let c1 = rsscommand_from_params(&s1);
        match c1 {
            Some(RssCommand::Add(u)) => {
                assert_eq!(u, u1);
            }
            _ => {
                assert!(false);
            }
        }

        let u2 = "this is not a URL".to_string();
        let s2 = format!("add {}", u2);
        let c2 = rsscommand_from_params(&s2);
        assert!(c2.is_none());

        let u3 = "file:///root/.ssh/id_rsa".to_string();
        let s3 = format!("add {}", u3);
        let c3 = rsscommand_from_params(&s3);
        assert!(c3.is_none());
    }

    #[test]
    fn rss_command_parsing_remove() {
        let s1 = "remove 3";
        let c1 = rsscommand_from_params(s1);
        match c1 {
            Some(RssCommand::Remove(i)) => {
                assert_eq!(i, 3);
            }
            _ => {
                assert!(false);
            }
        }

        let s2 = "remove NaN";
        let c2 = rsscommand_from_params(s2);
        assert!(c2.is_none());
    }

    #[test]
    fn rss_command_parsing_list() {
        let s1 = "list";
        let c1 = rsscommand_from_params(s1);
        match c1 {
            Some(RssCommand::List) => assert!(true),
            _ => assert!(false),
        }
    }

    #[test]
    fn rss_command_parsing_nocommand() {
        let s1 = "Just a line";
        let c1 = rsscommand_from_params(s1);
        assert!(c1.is_none());

        let s2 = ".rss just nonsense";
        let c2 = rsscommand_from_params(s2);
        assert!(c2.is_none());
    }

    #[test]
    fn rss_db_open() {
        let c = open_db(true);
        assert!(c.is_ok());
    }

    fn rss_add_example_feed(conn: &rusqlite::Connection, target: &IrcChannel) {
        const TESTFEED: &str = r#"<feed>
            <id>
            https://example.com/rss
            </id>
            <title>T-botti test feed</title>
            <updated>2021-01-26T11:31:04.605378+00:00</updated>
            <entry>
            <id>
            b07d6462374b97fe6fd03e665ec1fe84107d70989bff8408467805b076b58a0b
            </id>
            <title>Test entry 01</title>
            <updated>2021-01-26T11:31:04.605408+00:00</updated>
            <link href="https://example.com/testpost01" rel="alternate"/>
            </entry>
            </feed>"#;

        let feedurl = "https://example.com/rss";
        let parsed = parse_feed(TESTFEED, feedurl).unwrap();

        add_feed_to_db(&conn, parsed, &target).unwrap();
    }
    #[test]
    fn rss_add_feed() {
        let conn = open_db(true).unwrap();
        let target = IrcChannel {
            network: "testnetwork".to_owned(),
            channel: "#testing".to_owned(),
        };
        rss_add_example_feed(&conn, &target);

        let feeds = get_feeds_for_channel(&conn, &target).unwrap();
        assert_eq!(feeds.len(), 1);
        assert_eq!(feeds[0].url, "https://example.com/rss");
        assert_eq!(feeds[0].title, "T-botti test feed");
        assert_eq!(feeds[0].target.network, "testnetwork");
        assert_eq!(feeds[0].target.channel, "#testing");
        let feed_id = feeds[0].id;

        let entries = get_entries(&conn, feed_id).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].url, "https://example.com/testpost01");
        assert_eq!(entries[0].title, "Test entry 01");
    }

    #[tokio::test]
    async fn rss_list_feeds() {
        let (bot_tx, mut bot_rx) = mpsc::channel(10);
        let target = IrcChannel {
            network: "testnetwork".to_owned(),
            channel: "#testing".to_owned(),
        };
        let conn = open_db(true).unwrap();
        rss_add_example_feed(&conn, &target);

        let feeds = get_feeds_for_channel(&conn, &target).unwrap();
        list_feeds(bot_tx, &target, feeds).await;

        if let Some(msg) = bot_rx.recv().await {
            assert_eq!(
                msg,
                BotAction {
                    target: target,
                    action_type: ActionType::Message(
                        "1: T-botti test feed | https://example.com/rss".to_owned()
                    ),
                }
            );
        } else {
            assert!(false);
        }
    }

    #[tokio::test]
    async fn rss_remove_feed() {
        let target = IrcChannel {
            network: "testnetwork".to_owned(),
            channel: "#testing".to_owned(),
        };
        let conn = open_db(true).unwrap();
        rss_add_example_feed(&conn, &target);

        let feeds_before = get_feeds_for_channel(&conn, &target).unwrap();
        assert_eq!(feeds_before.len(), 1);

        let wrong_channel = IrcChannel {
            network: "secondnetwork".to_owned(),
            channel: "#testing".to_owned(),
        };
        assert!(remove_feed(&conn, &wrong_channel, feeds_before[0].id).is_err());
        let feeds_after_wrong = get_feeds_for_channel(&conn, &target).unwrap();
        assert_eq!(feeds_after_wrong.len(), 1);

        assert!(remove_feed(&conn, &target, feeds_before[0].id).is_ok());
        let feeds_after = get_feeds_for_channel(&conn, &target).unwrap();
        assert_eq!(feeds_after.len(), 0);
    }
}
