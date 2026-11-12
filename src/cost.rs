//! What a search cost: bytes each query scanned (as AWS reports it) times the list price.

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Engine {
    Insights,
    Athena,
}

#[derive(Debug, Clone, Serialize)]
pub struct Scan {
    pub log_group: String,
    pub engine: Engine,
    pub region: String,
    pub scanned_bytes: f64,
    pub usd: f64,
}

const GIB: f64 = 1024.0 * 1024.0 * 1024.0;

/// USD per GiB scanned, AWS Pricing API list prices as of 2026-11-12.
/// ponytail: hardcoded table, unlisted regions fall back to us-east-1; add rows when a region's price differs
fn usd_per_gib(engine: Engine, region: &str) -> f64 {
    match (engine, region) {
        (Engine::Insights, "ap-northeast-1") => 0.0076,
        (Engine::Insights, _) => 0.005,
        (Engine::Athena, "ap-east-2") => 4.5 / 1024.0,
        (Engine::Athena, _) => 5.0 / 1024.0,
    }
}

impl Scan {
    pub fn new(log_group: &str, engine: Engine, region: &str, scanned_bytes: f64) -> Self {
        // Athena bills at least 10 MB per query
        let billed = match engine {
            Engine::Athena => scanned_bytes.max(10.0 * 1024.0 * 1024.0),
            Engine::Insights => scanned_bytes,
        };
        Self {
            log_group: log_group.to_string(),
            engine,
            region: region.to_string(),
            scanned_bytes,
            usd: billed / GIB * usd_per_gib(engine, region),
        }
    }
}

pub fn total_usd(scans: &[Scan]) -> f64 {
    scans.iter().map(|s| s.usd).sum()
}

/// One line for humans, e.g. "Cost: ~$0.0127 (pluto/production via Athena 2.52 GB)"
pub fn summary(scans: &[Scan]) -> String {
    let parts = scans
        .iter()
        .map(|s| {
            let engine = match s.engine {
                Engine::Insights => "Insights",
                Engine::Athena => "Athena",
            };
            format!("{} via {} {:.2} GB", s.log_group, engine, s.scanned_bytes / GIB)
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!("Cost: ~${:.4} ({})", total_usd(scans), parts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prices_by_engine_and_region() {
        assert!((Scan::new("g", Engine::Insights, "ap-east-2", GIB).usd - 0.005).abs() < 1e-12);
        assert!((Scan::new("g", Engine::Insights, "ap-northeast-1", 2.0 * GIB).usd - 0.0152).abs() < 1e-12);
        assert!((Scan::new("g", Engine::Athena, "ap-east-2", 1024.0 * GIB).usd - 4.5).abs() < 1e-9);
        // 10 MB minimum
        assert_eq!(
            Scan::new("g", Engine::Athena, "ap-east-2", 1.0).usd,
            Scan::new("g", Engine::Athena, "ap-east-2", 10.0 * 1024.0 * 1024.0).usd
        );
    }
}
