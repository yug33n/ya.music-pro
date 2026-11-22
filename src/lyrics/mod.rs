pub mod fetcher;
pub mod parser;

pub use fetcher::LyricsFetcher;
pub use parser::{find_active_line, parse_lrc};
