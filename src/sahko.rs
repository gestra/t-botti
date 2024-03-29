/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use chrono::prelude::*;
use std::sync::Arc;
use tokio::sync::mpsc;
use yaml_rust::Yaml;

use crate::botaction::{ActionType, BotAction};
use crate::http_client::HTTP_CLIENT;
use crate::IrcChannel;

async fn get_json(fingrid_api_key: &str) -> Result<(String, String), reqwest::Error> {
    let priceurl = "https://api.spot-hinta.fi/Today";
    let fingridurl = "https://api.fingrid.fi/v1/variable/event/json/192%2C193%2C194%2C209";

    let price_req = HTTP_CLIENT.get(priceurl).send(); //.await?.text().await?;
    let fingrid_req = HTTP_CLIENT
        .get(fingridurl)
        .header("x-api-key", fingrid_api_key)
        .send();

    let price_json = price_req.await?.text().await?;
    let fingrid_json = fingrid_req.await?.text().await?;

    Ok((price_json, fingrid_json))
}

struct ElecData {
    price: f64,
    consumption: f64,
    production: f64,
    importexport: f64,
    state: u64,
}

fn parse_json(price_json: &str, fingrid_json: &str) -> Result<ElecData, String> {
    let prices: serde_json::Value = match serde_json::from_str(price_json) {
        Ok(j) => j,
        Err(_) => {
            return Err("Error parsing JSON".to_owned());
        }
    };

    let hour = Local::now().hour();

    let price_with_tax = {
        if let Some(d) = prices.as_array() {
            let hourly = &d[hour as usize];
            hourly["PriceWithTax"].as_f64()
        } else {
            return Err("No price found".to_string());
        }
    };

    let fg: serde_json::Value = match serde_json::from_str(fingrid_json) {
        Ok(j) => j,
        Err(_) => {
            return Err("Error parsing JSON".to_owned());
        }
    };

    let mut consumption: Option<f64> = None;
    let mut production: Option<f64> = None;
    let mut importexport: Option<f64> = None;
    let mut state: Option<u64> = None;

    if let Some(d) = fg.as_array() {
        for info in d {
            match info["variable_id"].as_i64() {
                Some(192) => {
                    production = info["value"].as_f64();
                }
                Some(193) => {
                    consumption = info["value"].as_f64();
                }
                Some(194) => {
                    importexport = info["value"].as_f64();
                }
                Some(209) => {
                    state = info["value"].as_u64();
                }
                _ => {}
            }
        }
    }

    if let (Some(c), Some(p), Some(i), Some(s)) = (consumption, production, importexport, state) {
        Ok(ElecData {
            price: price_with_tax.unwrap() * 100.0,
            consumption: c,
            production: p,
            importexport: i,
            state: s,
        })
    } else {
        Err("Fingrid-tietojen hakemisessa virhe".to_string())
    }
}

fn generate_msg(data: ElecData) -> String {
    let state_msg = match data.state {
        1 => "",
        2 => " | Sähköjärjestelmän käyttötila: Sähköjärjestelmän käyttötilanne on heikentynyt. Sähkön riittävyys Suomessa on uhattuna (sähköpulan riski on suuri) tai voimajärjestelmä ei täytä käyttövarmuuskriteerejä",
        3 => " | Sähköjärjestelmän käyttötila: Sähköjärjestelmän käyttövarmuus on vaarassa. Sähkönkulutusta on kytketty irti voimajärjestelmän käyttövarmuuden turvaamiseksi (sähköpula) tai riski laajaan sähkökatkoon on huomattava.",
        4 => " | Sähköjärjestelmän käyttötila: Vakava laajaa osaa tai koko Suomea kattava häiriö.",
        5 => " | Sähköjärjestelmän käyttötila: Vakavan häiriön käytönpalautus on menossa.",
        _ => " | Sähköjärjestelmän käyttötila: Tuntematon",
    };
    format!(
        "Sähkön spot-hinta: {:.2} snt/kWh | Tuotanto: {} MW | Kulutus: {} MW | Tuonti-/vienti+: {} MW{}",
        data.price, data.production, data.consumption, data.importexport, state_msg
    )
}

pub async fn command_sahko(
    bot_sender: mpsc::Sender<BotAction>,
    source: IrcChannel,
    config: Arc<Yaml>,
) {
    let fingrid_apikey = match config["fingrid"]["apikey"].as_str() {
        Some(a) => a,
        _ => {
            return;
        }
    };

    let msg = if let Ok((price_json, fingrid_json)) = get_json(fingrid_apikey).await {
        match parse_json(&price_json, &fingrid_json) {
            Ok(data) => generate_msg(data),
            Err(_) => "Virhe datan haussa".to_owned(),
        }
    } else {
        "Virhe datan haussa".to_owned()
    };

    let action = BotAction {
        target: source,
        action_type: ActionType::Message(msg),
    };

    bot_sender.send(action).await.unwrap();
}
