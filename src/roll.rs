/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use rand::prelude::*;
use tokio::sync::mpsc;

use crate::botaction::{ActionType, BotAction};
use crate::IrcChannel;

fn split_params(params: &str) -> Result<(i64, i64), ()> {
    let mut iter = params.split_whitespace();
    if let Some(first_p) = iter.next() {
        if let Ok(min) = first_p.parse::<i64>() {
            if let Some(second_p) = iter.next() {
                if let Ok(max) = second_p.parse::<i64>() {
                    if iter.next().is_none() && min < max {
                        return Ok((min, max));
                    }
                }
            }
        }
    }

    Err(())
}

fn roll(min: i64, max: i64) -> i64 {
    let mut rng = thread_rng();
    rng.gen_range(min..=max)
}

pub async fn command_roll(bot_sender: mpsc::Sender<BotAction>, source: IrcChannel, params: &str) {
    let msg = match split_params(params) {
        Ok((min, max)) => {
            let rolled = roll(min, max);
            format!("{}", rolled)
        }
        Err(()) => "Usage: .roll <min> <max>".to_owned(),
    };
    let a = BotAction {
        target: source,
        action_type: ActionType::Message(msg),
    };
    bot_sender.send(a).await.unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roll_test() {
        for _ in 0..=100 {
            eprintln!("{}", roll(1, 10));
        }
    }

    #[test]
    fn roll_params() {
        assert_eq!(split_params(&"1 10"), Ok((1, 10)));
        assert_eq!(split_params(&"    1     10    "), Ok((1, 10)));
        assert_eq!(split_params(&"    -1     10    "), Ok((-1, 10)));
        assert_eq!(split_params(&"-10 1"), Ok((-10, 1)));
        assert_eq!(split_params(&"10 1"), Err(()));
        assert_eq!(split_params(&"10"), Err(()));
        assert_eq!(split_params(&"1 10 100"), Err(()));
        assert_eq!(split_params(&""), Err(()));
    }
}
