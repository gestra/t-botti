/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use crate::botaction::{ActionType, BotAction};
use crate::IrcChannel;
use irc::client::prelude::Prefix;
use rusqlite::{named_params, Connection, Result};
use tokio::sync::mpsc;

const DEFAULT_LOCATION: &str = "Helsinki";

pub async fn command_weatherset(
    bot_sender: mpsc::Sender<BotAction>,
    source: IrcChannel,
    prefix: Option<Prefix>,
    location: &str,
) {
    if let Some(Prefix::Nickname(nick, _, _)) = prefix {
        if let Ok(c) = open_db(false) {
            let message = match set_location(&c, &nick, &source.network, location) {
                Ok(()) => "Weather location set".to_owned(),
                Err(_) => "Database error".to_owned(),
            };

            let a = BotAction {
                target: source,
                action_type: ActionType::Message(message),
            };

            bot_sender.send(a).await.unwrap();
        }
    }
}

pub fn open_db(testing: bool) -> Result<Connection> {
    let conn = match testing {
        true => rusqlite::Connection::open(":memory:")?,
        false => rusqlite::Connection::open("weather_locations.db")?,
    };

    conn.execute(
        "CREATE TABLE IF NOT EXISTS locations (
            id INTEGER PRIMARY KEY,
            network TEXT NOT NULL,
            nick TEXT NOT NULL,
            location TEXT NOT NULL,
            UNIQUE(network, nick) ON CONFLICT REPLACE
        )",
        [],
    )?;

    Ok(conn)
}

fn get_stored_location(conn: &Connection, nick: &str, network: &str) -> Result<Option<String>> {
    let mut location = None;

    let mut statement =
        conn.prepare("SELECT location FROM locations WHERE nick = :nick AND network = :network")?;
    let params = named_params! {":nick": nick, ":network": network};
    let mut rows = statement.query(params)?;

    if let Some(row) = rows.next()? {
        if let Ok(l) = row.get(0) {
            location = Some(l);
        }
    }

    Ok(location)
}

pub fn get_location(prefix: &Option<Prefix>, network: &str) -> String {
    let mut stored_location = None;
    if let Some(Prefix::Nickname(nick, _, _)) = prefix {
        if let Ok(c) = open_db(false) {
            if let Ok(Some(l)) = get_stored_location(&c, nick, network) {
                stored_location = Some(l);
            }
        }
    }

    let location = match &stored_location {
        Some(s) => s,
        None => DEFAULT_LOCATION,
    };

    location.to_owned()
}

pub fn set_location(conn: &Connection, nick: &str, network: &str, location: &str) -> Result<()> {
    let mut statement = conn.prepare(
        "INSERT INTO locations (network, nick, location) VALUES (:network, :nick, :location)",
    )?;
    statement.execute(named_params! {
        ":network": network,
        ":nick": nick,
        ":location": location,
    })?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weatherdb_setget() {
        let conn = open_db(true).unwrap();

        let nick = "testnick";
        let network = "testnetwork";
        let network2 = "anothernetwork";
        let location = "helsinki";
        let location2 = "tampere";

        let pre_res = get_stored_location(&conn, &nick, &network);
        assert_eq!(pre_res, Ok(None));

        let set_res = set_location(&conn, &nick, &network, &location);
        assert_eq!(set_res, Ok(()));

        let get_res = get_stored_location(&conn, &nick, &network);
        assert_eq!(get_res, Ok(Some(location.to_owned())));

        let second_set = set_location(&conn, &nick, &network, &location2);
        assert_eq!(second_set, Ok(()));

        let second_get = get_stored_location(&conn, &nick, &network);
        assert_eq!(second_get, Ok(Some(location2.to_owned())));

        let diff_network = get_stored_location(&conn, &nick, &network2);
        assert_eq!(diff_network, Ok(None));
    }
}
