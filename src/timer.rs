/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use chrono::{DateTime, Duration, NaiveDateTime, Utc};

use irc::client::prelude::*;

use log::{debug, error, info};

use regex::Regex;

use tokio::sync::mpsc;
use tokio::time::sleep;

use crate::botaction::{ActionType, BotAction};
use crate::IrcChannel;

#[derive(Debug)]
pub struct TimerEvent {
    pub target: IrcChannel,
    pub message: String,
    pub time: Duration,
}

pub async fn command_pizza(
    bot_sender: mpsc::Sender<BotAction>,
    timer_sender: mpsc::Sender<TimerEvent>,
    source: IrcChannel,
    prefix: Option<Prefix>,
) {
    let mins = 12;
    let duration = Duration::minutes(mins);

    let msg_to_send = if let Some(Prefix::Nickname(nick, _user, _host)) = prefix {
        format!("Apua {}! Pikku pizza palaa!", nick)
    } else {
        "Apua! Pikku pizza palaa!".to_owned()
    };

    let confirmation_msg = format!("Huudan sitten {} minuutin päästä pizzasta.", mins);

    bot_sender
        .send(BotAction {
            target: IrcChannel {
                network: source.network.to_owned(),
                channel: source.channel.to_owned(),
            },
            action_type: ActionType::Message(confirmation_msg),
        })
        .await
        .unwrap();

    timer_sender
        .send(TimerEvent {
            target: source,
            message: msg_to_send,
            time: duration,
        })
        .await
        .unwrap();
}

pub async fn command_bigone(
    bot_sender: mpsc::Sender<BotAction>,
    timer_sender: mpsc::Sender<TimerEvent>,
    source: IrcChannel,
    prefix: Option<Prefix>,
) {
    let mins = 15;
    let duration = Duration::minutes(mins);

    let msg_to_send = if let Some(Prefix::Nickname(nick, _user, _host)) = prefix {
        format!("Apua {}! Iso pizza palaa!", nick)
    } else {
        "Apua! Iso pizza palaa!".to_owned()
    };

    let confirmation_msg = format!("Huudan sitten {} minuutin päästä pizzasta.", mins);

    bot_sender
        .send(BotAction {
            target: IrcChannel {
                network: source.network.to_owned(),
                channel: source.channel.to_owned(),
            },
            action_type: ActionType::Message(confirmation_msg),
        })
        .await
        .unwrap();

    timer_sender
        .send(TimerEvent {
            target: source,
            message: msg_to_send,
            time: duration,
        })
        .await
        .unwrap();
}
pub async fn command_timer(
    bot_sender: mpsc::Sender<BotAction>,
    timer_sender: mpsc::Sender<TimerEvent>,
    source: IrcChannel,
    params: &str,
    prefix: Option<Prefix>,
) {
    lazy_static! {
        static ref RE_HHMM: Regex =
            Regex::new(r"^(?:(?P<hour>\d\d?)[:\.](?P<minute>\d\d))$").unwrap();
        static ref RE_HMS: Regex =
            Regex::new(r"^(?:(?P<hour>\d+)h)?(?:(?P<minute>\d+)(?:m|min))?(?:(?P<second>\d+)s)?$")
                .unwrap();
        static ref RE_MINUTES: Regex = Regex::new(r"^(?:(?P<minute>\d+))?$").unwrap();
    }

    let mut time_part = String::new();
    let mut message_part = String::new();
    let mut processing_time = true;
    for c in params.chars() {
        if processing_time {
            if !c.is_whitespace() {
                time_part.push(c);
            } else {
                processing_time = false;
            }
        } else {
            message_part.push(c);
        }
    }

    let duration;

    if RE_HHMM.is_match(&time_part) {
        let captures = RE_HHMM.captures(&time_part).unwrap();
        let hour = captures
            .name("hour")
            .map(|h| h.as_str().parse::<u32>().unwrap())
            .unwrap();
        let minute = captures
            .name("minute")
            .map(|h| h.as_str().parse::<u32>().unwrap())
            .unwrap();

        let now = chrono::Local::now();
        let today = now.date_naive();

        if let Some(mut timer_datetime) = today.and_hms_opt(hour, minute, 0) {
            let diff = timer_datetime - now.naive_local();
            if diff < Duration::seconds(0) {
                let one_day = Duration::days(1);
                timer_datetime += one_day;
            }

            duration = timer_datetime - now.naive_local();
        } else {
            bot_sender
                .send(BotAction {
                    target: source,
                    action_type: ActionType::Message(format!(
                        "Unable to parse time from {}",
                        time_part
                    )),
                })
                .await
                .unwrap();
            return;
        }
    } else if RE_HMS.is_match(&time_part) {
        let captures = RE_HMS.captures(&time_part).unwrap();
        let mut dur = Duration::seconds(0);
        if let Some(hour) = captures
            .name("hour")
            .map(|h| h.as_str().parse::<i64>().unwrap())
        {
            dur = dur + Duration::hours(hour);
        }
        if let Some(minute) = captures
            .name("minute")
            .map(|h| h.as_str().parse::<i64>().unwrap())
        {
            dur = dur + Duration::minutes(minute);
        }
        if let Some(second) = captures
            .name("second")
            .map(|h| h.as_str().parse::<i64>().unwrap())
        {
            dur = dur + Duration::seconds(second);
        }

        duration = dur;
    } else if RE_MINUTES.is_match(&time_part) {
        let captures = RE_MINUTES.captures(&time_part).unwrap();
        let minute = captures
            .name("minute")
            .map(|h| h.as_str().parse::<i64>().unwrap())
            .unwrap();
        duration = Duration::minutes(minute);
    } else {
        return;
    }

    if duration.num_seconds() < 0 {
        bot_sender
            .send(BotAction {
                target: source,
                action_type: ActionType::Message(
                    "Time parser failed: negative duration.".to_owned(),
                ),
            })
            .await
            .unwrap();
        return;
    }

    let msg_to_send = if let Some(Prefix::Nickname(nick, _user, _host)) = prefix {
        format!("{}: {}", nick, message_part)
    } else {
        format!("Timer: {}", message_part)
    };

    let total_secs = duration.num_seconds();
    let s = total_secs % 60;
    let m_temp = total_secs / 60;
    let m = m_temp % 60;
    let h = m_temp / 60;

    let mut confirmation_msg = "Huudan sitten ".to_owned();
    if h > 0 {
        let h_str = format!("{}h", h);
        confirmation_msg.push_str(&h_str);
    }
    if m > 0 {
        let m_str = format!("{}m", m);
        confirmation_msg.push_str(&m_str);
    }
    if s > 0 {
        let s_str = format!("{}s", s);
        confirmation_msg.push_str(&s_str);
    }
    confirmation_msg.push_str(" päästä asiasta.");

    bot_sender
        .send(BotAction {
            target: IrcChannel {
                network: source.network.to_owned(),
                channel: source.channel.to_owned(),
            },
            action_type: ActionType::Message(confirmation_msg),
        })
        .await
        .unwrap();

    timer_sender
        .send(TimerEvent {
            target: source,
            message: msg_to_send,
            time: duration,
        })
        .await
        .unwrap();
}

fn open_db() -> rusqlite::Result<rusqlite::Connection> {
    let conn = rusqlite::Connection::open("db/timer.db")?;
    conn.execute(
        "CREATE TABLE IF NOT EXISTS timers (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            time INTEGER NOT NULL,
            message TEXT,
            channel TEXT NOT NULL,
            network TEXT NOT NULL
        )",
        [],
    )?;

    Ok(conn)
}

fn remove_old_timers(conn: &rusqlite::Connection) -> rusqlite::Result<()> {
    let now = Utc::now().timestamp();
    let mut statement = conn.prepare("DELETE FROM timers WHERE time < :now")?;
    let params = rusqlite::named_params! {":now": now};
    let res = statement.execute(params);
    match res {
        Ok(n) => {
            info!("Removed {} old timers from db", n);
        }
        Err(e) => {
            error!("Error removing old timers from db: {:?}", e);
            return Err(e);
        }
    }

    Ok(())
}

fn get_timers_from_db(conn: &rusqlite::Connection) -> rusqlite::Result<Vec<(i64, TimerEvent)>> {
    let mut statement = conn.prepare("SELECT * FROM timers")?;
    let mut rows = statement.query([])?;

    let mut results = Vec::new();

    while let Some(row) = rows.next()? {
        let id: i64 = row.get(0)?;
        let timestamp: i64 = row.get(1)?;
        let message: String = row.get(2)?;
        let channel: String = row.get(3)?;
        let network: String = row.get(4)?;

        let target_dt = DateTime::<Utc>::from_utc(
            NaiveDateTime::from_timestamp_opt(timestamp, 0).unwrap(),
            Utc,
        );
        let now = Utc::now();
        let time = target_dt - now;

        let target = IrcChannel { channel, network };

        let event = TimerEvent {
            target,
            message,
            time,
        };
        results.push((id, event));
    }

    Ok(results)
}

fn remove_from_db(conn: &rusqlite::Connection, id: i64) -> rusqlite::Result<()> {
    let mut statement = conn.prepare("DELETE FROM timers WHERE id = :id")?;
    let res = statement.execute(rusqlite::named_params! {":id": id});

    match res {
        Ok(_) => {
            debug!("Removed timer id {} from db", id);
        }
        Err(e) => {
            error!("Error removing timer id {} from db: {:?}", id, e);
            return Err(e);
        }
    }

    Ok(())
}

fn start_timer(event: TimerEvent, sender: mpsc::Sender<BotAction>, db_id: Option<i64>) {
    let action = BotAction {
        target: event.target,
        action_type: ActionType::Message(event.message),
    };
    let time = event.time;
    tokio::spawn(async move {
        sleep(time.to_std().unwrap()).await;
        sender.send(action).await.unwrap();
        if let Some(id) = db_id {
            if let Ok(conn) = open_db() {
                remove_from_db(&conn, id).unwrap();
            }
        }
    });
}

fn add_timer_to_db(conn: &rusqlite::Connection, event: &TimerEvent) -> rusqlite::Result<i64> {
    let dt = Utc::now() + event.time;
    let timestamp = dt.timestamp();
    let message = event.message.to_owned();
    let channel = event.target.channel.to_owned();
    let network = event.target.network.to_owned();

    let mut statement = conn.prepare("INSERT INTO timers (time, message, channel, network) VALUES (:time, :message, :channel, :network)")?;
    let id = statement.insert(rusqlite::named_params! {
        ":time": timestamp,
        ":message": message,
        ":channel": channel,
        ":network": network,
    });

    debug!(
        "Added timer to db: {} {} {} {}",
        timestamp, message, channel, network
    );
    id
}

pub async fn timer_manager(
    mut receiver: mpsc::Receiver<TimerEvent>,
    sender: mpsc::Sender<BotAction>,
) {
    let db_conn = open_db();

    if let Ok(c) = &db_conn {
        let _ = remove_old_timers(c);

        if let Ok(old_timers) = get_timers_from_db(c) {
            info!("Adding {} old timers from db", old_timers.len());
            for (id, event) in old_timers {
                let new_sender = sender.clone();
                start_timer(event, new_sender, Some(id));
            }
        }
    } else {
        error!("Could not open timer db");
    }

    while let Some(event) = receiver.recv().await {
        let mut id = None;
        if let Ok(c) = &db_conn {
            let r = add_timer_to_db(c, &event);
            match r {
                Ok(i) => {
                    id = Some(i);
                }
                Err(_) => {
                    error!("Error when adding timer to db: {:?}", r);
                }
            }
        }
        let new_sender = sender.clone();
        start_timer(event, new_sender, id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::prelude::*;

    #[tokio::test]
    async fn timer_hhmm() {
        let (timer_tx, mut timer_rx) = mpsc::channel(10);
        let (bot_tx, _bot_rx) = mpsc::channel(10);

        let now = chrono::Local::now();
        let after_one_hour = now + Duration::hours(1);

        let time = after_one_hour.time();
        let params = format!("{}:{:02} moi", time.hour(), time.minute());

        command_timer(
            bot_tx,
            timer_tx,
            IrcChannel {
                network: "testnetwork".to_owned(),
                channel: "#testing".to_owned(),
            },
            &params,
            Some(Prefix::Nickname(
                "testnick".to_owned(),
                "testuser".to_owned(),
                "testhost".to_owned(),
            )),
        )
        .await;

        if let Some(result) = timer_rx.recv().await {
            assert_eq!(result.target.channel, "#testing".to_owned());
            assert_eq!(result.target.network, "testnetwork".to_owned().to_owned());
            assert_eq!(result.message, "testnick: moi".to_owned());
            assert!((result.time - Duration::hours(1)).num_seconds().abs() < 60);
        } else {
            assert!(false);
        }

        let (timer_tx, mut timer_rx) = mpsc::channel(10);
        let (bot_tx, mut bot_rx) = mpsc::channel(10);

        command_timer(
            bot_tx,
            timer_tx,
            IrcChannel {
                channel: "#testing".to_owned(),
                network: "testnetwork".to_owned(),
            },
            "36:90 mahotonta meininkiä",
            Some(Prefix::Nickname(
                "testnick".to_owned(),
                "testuser".to_owned(),
                "testhost".to_owned(),
            )),
        )
        .await;

        if let Some(_result) = timer_rx.recv().await {
            assert!(false);
        } else {
            if let Some(action) = bot_rx.recv().await {
                assert_eq!(action.target.channel, "#testing".to_owned());
                assert_eq!(
                    action.action_type,
                    ActionType::Message("Unable to parse time from 36:90".to_owned())
                );
            } else {
                assert!(false);
            }
        }
    }

    #[tokio::test]
    async fn timer_hms() {
        let (timer_tx, mut timer_rx) = mpsc::channel(10);
        let (bot_tx, _bot_rx) = mpsc::channel(10);

        command_timer(
            bot_tx,
            timer_tx,
            IrcChannel {
                channel: "#testing".to_owned(),
                network: "testnetwork".to_owned(),
            },
            "1h50m2s testing hms",
            Some(Prefix::Nickname(
                "testnick".to_owned(),
                "testuser".to_owned(),
                "testhost".to_owned(),
            )),
        )
        .await;

        if let Some(result) = timer_rx.recv().await {
            assert_eq!(result.target.channel, "#testing".to_owned().to_owned());
            assert_eq!(result.message, "testnick: testing hms".to_owned());
            assert_eq!(
                result.time,
                Duration::hours(1) + Duration::minutes(50) + Duration::seconds(2)
            );
        } else {
            assert!(false);
        }

        let (timer_tx, mut timer_rx) = mpsc::channel(10);
        let (bot_tx, _bot_rx) = mpsc::channel(10);

        command_timer(
            bot_tx,
            timer_tx,
            IrcChannel {
                channel: "#testing".to_owned(),
                network: "testnetwork".to_owned(),
            },
            "2s testing hms",
            Some(Prefix::Nickname(
                "testnick".to_owned(),
                "testuser".to_owned(),
                "testhost".to_owned(),
            )),
        )
        .await;

        if let Some(result) = timer_rx.recv().await {
            assert_eq!(result.target.channel, "#testing".to_owned().to_owned());
            assert_eq!(result.message, "testnick: testing hms".to_owned());
            assert_eq!(result.time, Duration::seconds(2));
        } else {
            assert!(false);
        }

        let (timer_tx, mut timer_rx) = mpsc::channel(10);
        let (bot_tx, _bot_rx) = mpsc::channel(10);

        command_timer(
            bot_tx,
            timer_tx,
            IrcChannel {
                channel: "#testing".to_owned(),
                network: "testnetwork".to_owned(),
            },
            "3h testing hms",
            Some(Prefix::Nickname(
                "testnick".to_owned(),
                "testuser".to_owned(),
                "testhost".to_owned(),
            )),
        )
        .await;

        if let Some(result) = timer_rx.recv().await {
            assert_eq!(result.target.channel, "#testing".to_owned().to_owned());
            assert_eq!(result.message, "testnick: testing hms".to_owned());
            assert_eq!(result.time, Duration::hours(3));
        } else {
            assert!(false);
        }

        let (timer_tx, mut timer_rx) = mpsc::channel(10);
        let (bot_tx, _bot_rx) = mpsc::channel(10);

        command_timer(
            bot_tx,
            timer_tx,
            IrcChannel {
                channel: "#testing".to_owned(),
                network: "testnetwork".to_owned(),
            },
            "3h36s testing hms",
            Some(Prefix::Nickname(
                "testnick".to_owned(),
                "testuser".to_owned(),
                "testhost".to_owned(),
            )),
        )
        .await;

        if let Some(result) = timer_rx.recv().await {
            assert_eq!(result.target.channel, "#testing".to_owned().to_owned());
            assert_eq!(result.message, "testnick: testing hms".to_owned());
            assert_eq!(result.time, Duration::hours(3) + Duration::seconds(36));
        } else {
            assert!(false);
        }
    }

    #[tokio::test]
    async fn timer_minutes() {
        let (timer_tx, mut timer_rx) = mpsc::channel(10);
        let (bot_tx, _bot_rx) = mpsc::channel(10);

        command_timer(
            bot_tx,
            timer_tx,
            IrcChannel {
                channel: "#testing".to_owned(),
                network: "testnetwork".to_owned(),
            },
            "60 testing just minutes",
            Some(Prefix::Nickname(
                "testnick".to_owned(),
                "testuser".to_owned(),
                "testhost".to_owned(),
            )),
        )
        .await;

        if let Some(result) = timer_rx.recv().await {
            assert_eq!(result.target.channel, "#testing".to_owned().to_owned());
            assert_eq!(result.message, "testnick: testing just minutes".to_owned());
            assert_eq!(result.time, Duration::hours(1));
        } else {
            assert!(false);
        }
    }
}
