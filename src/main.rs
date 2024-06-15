use color_eyre::{eyre::format_err, Report, Result};
use cron::Schedule;
use dotenv::dotenv;
use reqwest::Client;
use scraper::{Html, Selector};
use std::{str::FromStr, time::Duration};

#[tokio::main]
async fn main() -> Result<()> {
    let _ = dotenv();

    let product_url = std::env::var("PRODUCT_URL").expect("PRODUCT_URL must be set.");
    let cron = std::env::var("CRON").expect("CRON must be set.");

    let schedule = Schedule::from_str(&cron)?;

    let client = reqwest::Client::builder()
        // .timeout(Duration::from_secs(10))
        .gzip(true)
        .build()?;

    println!("Checking {} every {}", product_url, cron);

    // (IS_IN_ACTION, IS_IN_STOCK)
    let mut last_state = (false, false);

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

        let product_data: std::result::Result<(String, f32, Option<f32>), Report> =
            get_product_data(&product_url).await;
        let product_data = match product_data {
            Ok(sale) => sale,
            Err(e) => {
                println!("Error product data: {}", e);
                match send_error_mail(&client, e).await {
                    Ok(_) => {}
                    Err(e) => println!("Error sending error mail: {}", e),
                }
                continue;
            }
        };

        let special_sale = get_special_sale(&product_url).await;
        let special_sale = match special_sale {
            Ok(sale) => sale,
            Err(e) => {
                println!("Error: {}", e);
                match send_error_mail(&client, e).await {
                    Ok(_) => {}
                    Err(e) => println!("Error sending error mail: {}", e),
                }
                continue;
            }
        };

        let in_stock = get_stock(&product_url).await;
        let in_stock = match in_stock {
            Ok(sale) => sale,
            Err(e) => {
                println!("Error: {}", e);
                match send_error_mail(&client, e).await {
                    Ok(_) => {}
                    Err(e) => println!("Error sending error mail: {}", e),
                }
                continue;
            }
        };

        // If there is a discount
        match last_state {
            (true, true) => {}
            _ => {
                if in_stock {
                    match product_data.2 {
                        Some(old_price) => {
                            println!("Sale found: {:?}", product_data);

                            match send_mail(
                                &client,
                                (product_data.0.to_owned(), product_data.1, old_price),
                            )
                            .await
                            {
                                Ok(_) => {}
                                Err(e) => println!("Error sending mail: {}", e),
                            }
                        }
                        None => {
                            println!("No sale found.");
                        }
                    };

                    match &special_sale {
                        Some(special_sale) => {
                            println!("Special sale found: {:?}", special_sale);

                            match send_special_sale_mail(&client, product_data.0).await {
                                Ok(_) => {}
                                Err(e) => println!("Error sending mail: {}", e),
                            }
                        }
                        None => {
                            println!("No special sale found.");
                        }
                    }
                }
            }
        }

        last_state = (product_data.2.is_some() || special_sale.is_some(), in_stock);
    }
}

async fn get_special_sale(product_url: &str) -> Result<Option<String>> {
    println!("Getting special sale for {}", product_url);

    let product_id = product_url
        .split('/')
        .last()
        .ok_or_else(|| format_err!("Invalid product url"))?;
    let url = format!("https://www.kruidvat.nl/view/PromotionBoxComponentController?componentUid=PromotionBoxComponent&currentProductCode={}", product_id);

    let mut res = surf::get(url).await.map_err(|e| format_err!("{}", e))?;
    if !res.status().is_success() {
        return Err(format_err!(
            "Failed to get special sale url: {}",
            res.status()
        ));
    }

    let body = res.body_string().await.map_err(|e| format_err!("{}", e))?;
    if body.is_empty() {
        return Ok(None);
    }

    let document = Html::parse_document(&body);

    let sale_name = document
        .select(&Selector::parse(".promotion-box__information-text").unwrap())
        .next()
        .ok_or_else(|| format_err!("Failed to find sale name"))?
        .text()
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_owned();

    Ok(Some(sale_name))
}

async fn get_stock(product_url: &str) -> Result<bool> {
    println!("Getting stock for {}", product_url);

    let product_id = product_url
        .split('/')
        .last()
        .ok_or_else(|| format_err!("Invalid product url"))?;
    let url = format!(
        "https://www.kruidvat.nl/api/v2/kvn/products/{}/onlinestock",
        product_id
    );

    let mut res = surf::get(url).await.map_err(|e| format_err!("{}", e))?;
    if !res.status().is_success() {
        return Err(format_err!(
            "Failed to get special sale url: {}",
            res.status()
        ));
    }

    let body = res.body_string().await.map_err(|e| format_err!("{}", e))?;

    let stock: serde_json::Value = serde_json::from_str(&body)?;
    let stock = stock["stockLevelStatus"]
        .as_str()
        .ok_or_else(|| format_err!("Failed to parse stock: {}", stock))?;

    let in_stock = stock != "outOfStock";

    Ok(in_stock)
}

async fn get_product_data(product_url: &str) -> Result<(String, f32, Option<f32>)> {
    println!("Getting sale for {}", product_url);

    let mut res = surf::get(product_url)
        .await
        .map_err(|e| format_err!("{}", e))?;
    if !res.status().is_success() {
        return Err(format_err!("Failed to get product page: {}", res.status()));
    }

    let html = res.body_string().await.map_err(|e| format_err!("{}", e))?;
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
        Some(old_price) => Some(
            old_price
                .text()
                .collect::<Vec<_>>()
                .join("")
                .replace("\n", "")
                .replace(" ", "")
                .replace(",", ".")
                .to_string()
                .parse::<f32>()?,
        ),
        None => None,
    };

    Ok((name, price, old_price))
}

async fn send_special_sale_mail(client: &Client, product_name: String) -> Result<()> {
    let mailgun_api_key = std::env::var("MAILGUN_API_KEY").expect("MAILGUN_API_KEY must be set.");
    let mailgun_domain = std::env::var("MAILGUN_DOMAIN").expect("MAILGUN_DOMAIN must be set.");
    let mailgun_from = std::env::var("MAILGUN_FROM").expect("MAILGUN_FROM must be set.");
    let mailgun_to = std::env::var("MAILGUN_TO").expect("MAILGUN_TO must be set.");

    let product_url = std::env::var("PRODUCT_URL").expect("PRODUCT_URL must be set.");

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
                format!("{} is in de actie", product_name).to_owned(),
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

async fn send_mail(client: &Client, data: (String, f32, f32)) -> Result<()> {
    let mailgun_api_key = std::env::var("MAILGUN_API_KEY").expect("MAILGUN_API_KEY must be set.");
    let mailgun_domain = std::env::var("MAILGUN_DOMAIN").expect("MAILGUN_DOMAIN must be set.");
    let mailgun_from = std::env::var("MAILGUN_FROM").expect("MAILGUN_FROM must be set.");
    let mailgun_to = std::env::var("MAILGUN_TO").expect("MAILGUN_TO must be set.");

    let product_url = std::env::var("PRODUCT_URL").expect("PRODUCT_URL must be set.");

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
                    data.0, data.2, data.1,
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

async fn send_error_mail(client: &Client, data: Report) -> Result<()> {
    let mailgun_api_key = std::env::var("MAILGUN_API_KEY").expect("MAILGUN_API_KEY must be set.");
    let mailgun_domain = std::env::var("MAILGUN_DOMAIN").expect("MAILGUN_DOMAIN must be set.");
    let mailgun_from = std::env::var("MAILGUN_FROM").expect("MAILGUN_FROM must be set.");
    let mailgun_to = std::env::var("MAILGUN_ERROR_TO").expect("MAILGUN_ERROR_TO must be set.");

    let product_url = std::env::var("PRODUCT_URL").expect("PRODUCT_URL must be set.");

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
                format!("ERROR: kruidvat-notification failed for {}", product_url).to_owned(),
            ),
            ("text", format!("{}\n{}", product_url, data).to_owned()),
        ])
        .send()
        .await?;

    if !res.status().is_success() {
        return Err(format_err!("Failed to send mail: {}", res.status()));
    }

    println!("Error mail sent to {}", mailgun_to);

    Ok(())
}
