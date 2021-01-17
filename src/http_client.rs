/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use std::time::Duration;

lazy_static! {
    pub static ref HTTP_CLIENT: reqwest::Client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (X11; Fedora; Linux x86_64; rv:84.0) Gecko/20100101 Firefox/84.0")
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();
}

pub async fn get_url(url: &str) -> reqwest::Result<String> {
    let contents = HTTP_CLIENT.get(url).send().await?.text().await?;

    Ok(contents)
}
