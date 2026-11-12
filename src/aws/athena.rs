//! Search a CloudWatch log group's S3 archive (CloudWatch → Firehose → S3) with Athena,
//! for time ranges the group's retention no longer covers.
//! Expects the Firehose/Glue layout: partitions year/month/day/hour (int, UTC arrival
//! time), columns messagetype, logstream, logevents array<struct<id,timestamp,message>>.

use crate::aws::logs::sdk_error;
use crate::aws::{LogEntry, SearchParams};
use crate::config::AthenaTarget;
use anyhow::{anyhow, Result};
use aws_sdk_athena::types::QueryExecutionState;
use aws_sdk_athena::Client;
use chrono::{DateTime, Datelike, Duration, DurationRound, Timelike, Utc};
use tokio::time::sleep;

/// Partition predicate covering [start, end]. Partitions are the UTC hour S3 received
/// the batch, up to ~5 minutes after the log time: pad the end, plus a little slack.
/// Each extra hour is a full hour of scanning, so no blanket hour of padding.
fn partition_filter(start: DateTime<Utc>, end: DateTime<Utc>) -> String {
    let first = (start - Duration::minutes(5))
        .duration_trunc(Duration::hours(1))
        .unwrap();
    let last = end + Duration::minutes(10);
    let mut clauses = Vec::new();
    let mut day = first;
    while day <= last {
        let next_day = day
            .date_naive()
            .succ_opt()
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc();
        let (lo, hi) = (day.hour(), (next_day - Duration::hours(1)).min(last).hour());
        let hours = if lo == 0 && hi == 23 {
            String::new()
        } else {
            format!(" AND hour BETWEEN {} AND {}", lo, hi)
        };
        clauses.push(format!(
            "(year = {} AND month = {} AND day = {}{})",
            day.year(),
            day.month(),
            day.day(),
            hours
        ));
        day = next_day;
    }
    clauses.join(" OR ")
}

fn sql_string(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// Same semantics as the Insights query: patterns are regexes, ANDed; excludes NOT-ed.
/// DISTINCT drops batches Firehose delivered twice.
fn build_query(table: &str, params: &SearchParams, start: DateTime<Utc>, end: DateTime<Utc>) -> String {
    let mut conditions = vec![
        format!("({})", partition_filter(start, end)),
        "messagetype = 'DATA_MESSAGE'".to_string(),
        format!(
            "e.\"timestamp\" BETWEEN {} AND {}",
            start.timestamp_millis(),
            end.timestamp_millis()
        ),
    ];
    for p in &params.patterns {
        conditions.push(format!("regexp_like(e.message, {})", sql_string(p)));
    }
    for p in &params.exclude {
        conditions.push(format!("NOT regexp_like(e.message, {})", sql_string(p)));
    }
    format!(
        "SELECT DISTINCT e.\"timestamp\", logstream, e.message\nFROM {}\nCROSS JOIN UNNEST(logevents) AS u(e)\nWHERE {}\nORDER BY 1 DESC\nLIMIT {}",
        table,
        conditions.join("\n  AND "),
        params.limit
    )
}

async fn stop(client: &Client, id: &str) -> anyhow::Error {
    match client.stop_query_execution().query_execution_id(id).send().await {
        Ok(_) => anyhow!("cancelled (Athena query {} stopped)", id),
        Err(e) => anyhow!("cancelled, but StopQueryExecution failed for {}: {}", id, sdk_error(e)),
    }
}

pub async fn search(
    config: &aws_config::SdkConfig,
    group: &str,
    target: &AthenaTarget,
    params: &SearchParams,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Result<(Vec<LogEntry>, f64)> {
    let client = Client::new(config);

    let query = build_query(&target.table, params, start, end);
    if std::env::var("LOG_HOUND_DEBUG").is_ok() {
        eprintln!("DEBUG: Athena {} ({}):\n{}", target.table, target.workgroup, query);
    }

    let id = client
        .start_query_execution()
        .work_group(&target.workgroup)
        .query_string(&query)
        .send()
        .await
        .map_err(sdk_error)?
        .query_execution_id
        .ok_or_else(|| anyhow!("No query execution ID returned"))?;

    // Same Ctrl-C contract as Insights: stop the query on AWS, don't leave it scanning
    let ctrl_c = tokio::signal::ctrl_c();
    tokio::pin!(ctrl_c);
    let scanned = loop {
        let response = tokio::select! {
            r = client.get_query_execution().query_execution_id(&id).send() => r.map_err(sdk_error)?,
            _ = &mut ctrl_c => return Err(stop(&client, &id).await),
        };
        let execution = response.query_execution();
        let status = execution.and_then(|q| q.status());
        match status.and_then(|s| s.state()) {
            Some(QueryExecutionState::Succeeded) => {
                break execution
                    .and_then(|q| q.statistics())
                    .and_then(|s| s.data_scanned_in_bytes())
                    .unwrap_or(0) as f64
            }
            Some(QueryExecutionState::Failed) | Some(QueryExecutionState::Cancelled) => {
                // e.g. the workgroup's per-query scan cap: narrow the range
                let reason = status.and_then(|s| s.state_change_reason()).unwrap_or("no reason given");
                return Err(anyhow!("Athena query {}: {}", id, reason));
            }
            _ => tokio::select! {
                _ = sleep(std::time::Duration::from_secs(1)) => {}
                _ = &mut ctrl_c => return Err(stop(&client, &id).await),
            },
        }
    };

    let mut entries = Vec::new();
    let mut token = None;
    let mut header = true;
    loop {
        let response = client
            .get_query_results()
            .query_execution_id(&id)
            .set_next_token(token)
            .send()
            .await
            .map_err(sdk_error)?;
        for row in response.result_set().map(|r| r.rows()).unwrap_or_default() {
            if std::mem::take(&mut header) {
                continue; // first row of the first page is the column names
            }
            let cells: Vec<Option<&str>> = row.data().iter().map(|d| d.var_char_value()).collect();
            if let [Some(ts), stream, Some(message)] = cells[..] {
                let Some(timestamp) = ts.parse().ok().and_then(DateTime::from_timestamp_millis) else {
                    continue;
                };
                entries.push(LogEntry {
                    timestamp,
                    message: message.to_string(),
                    log_group: group.to_string(),
                    log_stream: stream.map(str::to_string),
                    region: Some(target.region.clone()),
                });
            }
        }
        token = response.next_token;
        if token.is_none() {
            break;
        }
    }
    Ok((entries, scanned))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn utc(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, mo, d, h, mi, 0).unwrap()
    }

    #[test]
    fn partitions_cover_arrival_delay_and_split_days() {
        assert_eq!(
            partition_filter(utc(2026, 10, 15, 2, 10), utc(2026, 10, 15, 2, 30)),
            "(year = 2026 AND month = 10 AND day = 15 AND hour BETWEEN 2 AND 2)"
        );
        // logged at 03:55 can land in the 04:00 partition
        assert_eq!(
            partition_filter(utc(2026, 10, 15, 2, 0), utc(2026, 10, 15, 3, 55)),
            "(year = 2026 AND month = 10 AND day = 15 AND hour BETWEEN 1 AND 4)"
        );
        assert_eq!(
            partition_filter(utc(2026, 10, 14, 23, 30), utc(2026, 10, 16, 0, 30)),
            "(year = 2026 AND month = 10 AND day = 14 AND hour BETWEEN 23 AND 23) OR \
             (year = 2026 AND month = 10 AND day = 15) OR \
             (year = 2026 AND month = 10 AND day = 16 AND hour BETWEEN 0 AND 0)"
        );
    }

    #[test]
    fn query_quotes_patterns() {
        let params = SearchParams::new(vec!["it's".into()], vec!["health".into()], 5);
        let q = build_query("db.t", &params, utc(2026, 10, 15, 2, 0), utc(2026, 10, 15, 3, 0));
        assert!(q.contains("regexp_like(e.message, 'it''s')"));
        assert!(q.contains("NOT regexp_like(e.message, 'health')"));
        assert!(q.ends_with("LIMIT 5"));
    }
}
