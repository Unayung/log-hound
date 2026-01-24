mod client;
mod logs;
mod multi_region;

pub use client::create_client;
pub use logs::{LogEntry, LogSearcher, MultiRegionSearcher};
pub use multi_region::RegionalLogGroup;
