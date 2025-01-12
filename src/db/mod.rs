mod migrations;
pub mod schema;
pub mod writer;

pub use migrations::run as setup_db;

use crate::{
    error::Error,
    logs::{
        schema::{ChannelLogDate, UserLogDate},
        stream::LogsStream,
    },
    web::schema::AvailableLogDate,
    Result,
};
use chrono::{Datelike, NaiveDateTime};
use clickhouse::Client;
use rand::{seq::IteratorRandom, thread_rng};
use tracing::info;

pub async fn read_channel(
    db: &Client,
    channel_id: &str,
    log_date: ChannelLogDate,
    reverse: bool,
    limit: Option<u64>,
    offset: Option<u64>,
) -> Result<LogsStream> {
    let suffix = if reverse { "DESC" } else { "ASC" };
    let mut query = format!("SELECT raw FROM message WHERE channel_id = ? AND toStartOfDay(timestamp) = ? ORDER BY timestamp {suffix}");
    apply_limit_offset(&mut query, limit, offset);

    let cursor = db
        .query(&query)
        .bind(channel_id)
        .bind(log_date.to_string())
        .fetch()?;
    LogsStream::new_cursor(cursor).await
}

pub async fn read_user(
    db: &Client,
    channel_id: &str,
    user_id: &str,
    log_date: UserLogDate,
    reverse: bool,
    limit: Option<u64>,
    offset: Option<u64>,
) -> Result<LogsStream> {
    let suffix = if reverse { "DESC" } else { "ASC" };
    let mut query = format!("SELECT raw FROM message WHERE channel_id = ? AND user_id = ? AND toStartOfMonth(timestamp) = ? ORDER BY timestamp {suffix}");
    apply_limit_offset(&mut query, limit, offset);

    let cursor = db
        .query(&query)
        .bind(channel_id)
        .bind(user_id)
        .bind(format!("{}-{:0>2}-1", log_date.year, log_date.month))
        .fetch()?;

    LogsStream::new_cursor(cursor).await
}

pub async fn read_available_channel_logs(
    db: &Client,
    channel_id: &str,
) -> Result<Vec<AvailableLogDate>> {
    let timestamps: Vec<i32> = db
        .query(
            "SELECT toDateTime(toStartOfDay(timestamp)) AS date FROM message WHERE channel_id = ? GROUP BY date ORDER BY date DESC",
        )
        .bind(channel_id)
        .fetch_all().await?;

    let dates = timestamps
        .into_iter()
        .map(|timestamp| {
            let naive =
                NaiveDateTime::from_timestamp_opt(timestamp.into(), 0).expect("Invalid DateTime");

            AvailableLogDate {
                year: naive.year().to_string(),
                month: naive.month().to_string(),
                day: Some(naive.day().to_string()),
            }
        })
        .collect();

    Ok(dates)
}

pub async fn read_available_user_logs(
    db: &Client,
    channel_id: &str,
    user_id: &str,
) -> Result<Vec<AvailableLogDate>> {
    let timestamps: Vec<i32> = db
        .query("SELECT toDateTime(toStartOfMonth(timestamp)) AS date FROM message WHERE channel_id = ? AND user_id = ? GROUP BY date ORDER BY date DESC")
        .bind(channel_id)
        .bind(user_id)
        .fetch_all().await?;

    let dates = timestamps
        .into_iter()
        .map(|timestamp| {
            let naive =
                NaiveDateTime::from_timestamp_opt(timestamp.into(), 0).expect("Invalid DateTime");

            AvailableLogDate {
                year: naive.year().to_string(),
                month: naive.month().to_string(),
                day: None,
            }
        })
        .collect();

    Ok(dates)
}

pub async fn read_random_user_line(db: &Client, channel_id: &str, user_id: &str) -> Result<String> {
    let total_count = db
        .query("SELECT count(*) FROM message WHERE channel_id = ? AND user_id = ? ")
        .bind(channel_id)
        .bind(user_id)
        .fetch_one::<u64>()
        .await?;

    if total_count == 0 {
        return Err(Error::NotFound);
    }

    let offset = {
        let mut rng = thread_rng();
        (0..total_count).choose(&mut rng).ok_or(Error::NotFound)
    }?;

    let text = db
        .query(
            "WITH
            (SELECT timestamp FROM message WHERE channel_id = ? AND user_id = ? LIMIT 1 OFFSET ?)
            AS random_timestamp
            SELECT raw FROM message WHERE channel_id = ? AND user_id = ? AND timestamp = random_timestamp",
        )
        .bind(channel_id)
        .bind(user_id)
        .bind(offset)
        .bind(channel_id)
        .bind(user_id)
        .fetch_optional::<String>()
        .await?
        .ok_or(Error::NotFound)?;

    Ok(text)
}

pub async fn read_random_channel_line(db: &Client, channel_id: &str) -> Result<String> {
    let total_count = db
        .query("SELECT count(*) FROM message WHERE channel_id = ? ")
        .bind(channel_id)
        .fetch_one::<u64>()
        .await?;

    if total_count == 0 {
        return Err(Error::NotFound);
    }

    let offset = {
        let mut rng = thread_rng();
        (0..total_count).choose(&mut rng).ok_or(Error::NotFound)
    }?;

    let text = db
        .query(
            "WITH
            (SELECT timestamp FROM message WHERE channel_id = ? LIMIT 1 OFFSET ?)
            AS random_timestamp
            SELECT raw FROM message WHERE channel_id = ? AND timestamp = random_timestamp",
        )
        .bind(channel_id)
        .bind(offset)
        .bind(channel_id)
        .fetch_optional::<String>()
        .await?
        .ok_or(Error::NotFound)?;

    Ok(text)
}

pub async fn delete_user_logs(db: &Client, user_id: &str) -> Result<()> {
    info!("Deleting all logs for user {user_id}");
    db.query("ALTER TABLE message DELETE WHERE user_id = ?")
        .bind(user_id)
        .execute()
        .await?;
    Ok(())
}

fn apply_limit_offset(query: &mut String, limit: Option<u64>, offset: Option<u64>) {
    if let Some(limit) = limit {
        *query = format!("{query} LIMIT {limit}");
    }
    if let Some(offset) = offset {
        *query = format!("{query} OFFSET {offset}");
    }
}
