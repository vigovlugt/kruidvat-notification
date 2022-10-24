use std::{str::FromStr, time::Duration};

use color_eyre::{eyre::format_err, Result};
use cron::Schedule;
use dotenv::dotenv;
use scraper::{Html, Selector};

#[tokio::main]
async fn main() -> Result<()> {
    let _ = dotenv();
    let product_url = std::env::var("PRODUCT_URL").expect("PRODUCT_URL must be set.");
    let cron = std::env::var("CRON").expect("CRON must be set.");
    println!("Checking {} every {}", product_url, cron);

    let schedule = Schedule::from_str(&cron)?;

    loop {
        tokio::time::sleep(Duration::from_secs(1)).await;

        let next = schedule.upcoming(chrono::Utc).next().unwrap();

        let now = chrono::Utc::now();

        if next > now {
            let duration = next - now;
            println!("Sleeping for {:?}", duration);
            tokio::time::sleep(Duration::from_secs(duration.num_seconds() as u64)).await;
        }

        println!("Checking for product availability");

        let sale = get_sale(&product_url).await?;

        let sale = match sale {
            Some(sale) => sale,
            None => {
                println!("No sale found.");
                continue;
            }
        };

        println!("Sale found: {:?}", sale);

        send_mail(sale).await?;
    }
}

async fn get_sale(product_url: &str) -> Result<Option<(String, f32, f32)>> {
    println!("Getting sale for {}", product_url);

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .gzip(true)
        .build()?;

    let res = client.get(product_url).send().await?;
    if !res.status().is_success() {
        return Err(format_err!("Failed to get product page: {}", res.status()));
    }

    let html = res.text().await?;
    let document = Html::parse_document(&html);

    let name = document
        .select(&Selector::parse(".title").unwrap())
        .next()
        .ok_or_else(|| format_err!("Failed to find product name"))?
        .text()
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_owned();

    let price = document
        .select(&Selector::parse(".pricebadge__new-price-wrapper").unwrap())
        .next()
        .ok_or_else(|| format_err!("Could not find price"))?
        .text()
        .collect::<Vec<_>>()
        .join("")
        .replace("\n", "")
        .replace(" ", "")
        .replace(",", ".")
        .to_string()
        .parse::<f32>()?;

    let old_price = document
        .select(&Selector::parse(".pricebadge__old-price-content").unwrap())
        .next();
    let old_price = match old_price {
        Some(old_price) => old_price,
        None => return Ok(None),
    };

    let old_price = old_price
        .text()
        .collect::<Vec<_>>()
        .join("")
        .replace("\n", "")
        .replace(" ", "")
        .replace(",", ".")
        .to_string()
        .parse::<f32>()?;

    return Ok(Some((name, price, old_price)));
}

async fn send_mail(data: (String, f32, f32)) -> Result<()> {
    let _ = dotenv();
    let mailgun_api_key = std::env::var("MAILGUN_API_KEY").expect("MAILGUN_API_KEY must be set.");
    let mailgun_domain = std::env::var("MAILGUN_DOMAIN").expect("MAILGUN_DOMAIN must be set.");
    let mailgun_from = std::env::var("MAILGUN_FROM").expect("MAILGUN_FROM must be set.");
    let mailgun_to = std::env::var("MAILGUN_TO").expect("MAILGUN_TO must be set.");

    let product_url = std::env::var("PRODUCT_URL").expect("PRODUCT_URL must be set.");

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .gzip(true)
        .build()?;

    let res = client
        .post(&format!(
            "https://api.mailgun.net/v3/{}/messages",
            mailgun_domain
        ))
        .basic_auth("api", Some(mailgun_api_key))
        .form(&[
            ("from", mailgun_from),
            ("to", mailgun_to.to_owned()),
            (
                "subject",
                format!(
                    "{} in de aanbieding van €{} voor €{}",
                    data.0, data.2, data.1
                )
                .to_owned(),
            ),
            ("text", format!("{}", product_url).to_owned()),
        ])
        .send()
        .await?;

    if !res.status().is_success() {
        return Err(format_err!("Failed to send mail: {}", res.status()));
    }

    println!("Mail sent to {}", mailgun_to);

    Ok(())
}
