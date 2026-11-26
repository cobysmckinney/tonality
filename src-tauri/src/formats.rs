//! The on-disk formats this build reads and writes, as the updater needs them: printed for the
//! release workflow to put in the update feed, and compared before an older build is offered.
//! See `tonality_core::format`.

use tonality_core::Formats;

/// This build's formats, as the JSON object the update feed carries under `tonality`.
pub fn json() -> String {
    serde_json::to_string(&Formats::CURRENT).expect("three numbers")
}

/// `text` as the same kind of value as `like` (a version, without naming its type here).
pub fn parsed_like<T: std::str::FromStr>(_like: &T, text: &str) -> Option<T> {
    text.parse().ok()
}

/// Whether the build a feed (`latest.json`) describes reads everything this build writes. The
/// feed says what its build reads under `tonality` (see `tonality_core::format`); one that
/// doesn't say (every release before this was added) is taken not to.
pub fn reads_this_library(feed: &serde_json::Value) -> bool {
    feed.get("tonality")
        .and_then(|f| serde_json::from_value::<Formats>(f.clone()).ok())
        .is_some_and(|theirs| theirs.reads(&Formats::CURRENT))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn an_older_build_is_only_offered_if_it_reads_this_library() {
        let ours = Formats::CURRENT;
        let feed = |f: Formats| json!({ "version": "0.1.1", "tonality": f, "platforms": {} });
        assert!(reads_this_library(&feed(ours)));
        assert!(reads_this_library(&feed(Formats {
            catalog: ours.catalog + 1,
            ..ours
        })));
        // Behind in any one of them: it would refuse the catalog, or lose settings.
        for behind in [
            Formats {
                catalog: ours.catalog - 1,
                ..ours
            },
            Formats {
                edits: ours.edits - 1,
                ..ours
            },
        ] {
            assert!(!reads_this_library(&feed(behind)), "{behind:?}");
        }
        // A release from before feeds said (v0.1.1 and earlier), or one that says nonsense.
        assert!(!reads_this_library(&json!({ "version": "0.1.1" })));
        assert!(!reads_this_library(
            &json!({ "version": "0.1.1", "tonality": "yes" })
        ));
    }

    #[test]
    fn the_formats_flag_prints_what_the_feed_carries() {
        let printed: serde_json::Value = serde_json::from_str(&json()).unwrap();
        assert!(reads_this_library(&json!({ "tonality": printed })));
    }
}
