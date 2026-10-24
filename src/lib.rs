use std::cmp::Ordering;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Match {
    pub row: usize,
    pub start: usize,
    pub end: usize,
}

#[derive(Clone, Debug, Default)]
pub struct Search {
    pub lines: Vec<String>,
    pub pattern: String,
    pub cursor_x: usize,
    pub cursor_y: usize,
}

fn lowercase(s: &str) -> String {
    s.chars().flat_map(char::to_lowercase).collect()
}

fn grapheme_width(s: &str) -> usize {
    UnicodeWidthStr::width(s)
}

pub fn column_at(line: &str, grapheme_index: usize) -> usize {
    line.graphemes(true)
        .take(grapheme_index)
        .map(grapheme_width)
        .sum()
}

impl Search {
    pub fn find_matches(&self) -> Vec<Match> {
        let needle: Vec<&str> = self.pattern.graphemes(true).collect();
        if needle.is_empty() {
            return Vec::new();
        }

        let case_insensitive = self.pattern.chars().all(|ch| !ch.is_uppercase());
        let needle: Vec<String> = needle
            .into_iter()
            .map(|cluster| {
                if case_insensitive {
                    lowercase(cluster)
                } else {
                    cluster.to_owned()
                }
            })
            .collect();
        let mut matches = Vec::new();

        for (row, line) in self.lines.iter().enumerate() {
            let haystack: Vec<String> = line
                .graphemes(true)
                .map(|cluster| {
                    if case_insensitive {
                        lowercase(cluster)
                    } else {
                        cluster.to_owned()
                    }
                })
                .collect();
            if needle.len() > haystack.len() {
                continue;
            }
            for start in 0..=haystack.len() - needle.len() {
                if haystack[start..start + needle.len()] == needle {
                    matches.push(Match {
                        row,
                        start,
                        end: start + needle.len(),
                    });
                }
            }
        }

        matches.sort_by(|a, b| {
            let a_y = a.row.abs_diff(self.cursor_y);
            let b_y = b.row.abs_diff(self.cursor_y);
            match a_y.cmp(&b_y) {
                Ordering::Equal => {
                    let a_x = column_at(&self.lines[a.row], a.start).abs_diff(self.cursor_x);
                    let b_x = column_at(&self.lines[b.row], b.start).abs_diff(self.cursor_x);
                    a_x.cmp(&b_x)
                }
                other => other,
            }
        });
        matches
    }

    pub fn assign_labels(&self, matches: &[Match], alphabet: &str) -> Vec<(char, Match)> {
        let case_insensitive = self.pattern.chars().all(|ch| !ch.is_uppercase());
        let mut continuations = Vec::new();
        for found in matches {
            let next = self.lines[found.row].graphemes(true).nth(found.end);
            if let Some(next) = next {
                continuations.push(if case_insensitive {
                    lowercase(next)
                } else {
                    next.to_owned()
                });
            }
        }

        alphabet
            .chars()
            .filter(|label| {
                let label = label.to_string();
                !continuations.iter().any(|next| {
                    if case_insensitive {
                        lowercase(&label) == *next
                    } else {
                        label == *next
                    }
                })
            })
            .zip(matches.iter().cloned())
            .collect()
    }
}

pub fn width(s: &str) -> usize {
    grapheme_width(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_smart_case_and_nearest_first() {
        let mut search = Search {
            lines: vec!["First foo".into(), "foo First".into()],
            pattern: "foo".into(),
            cursor_x: 4,
            cursor_y: 0,
        };
        let matches = search.find_matches();
        assert_eq!(
            matches,
            vec![
                Match {
                    row: 0,
                    start: 6,
                    end: 9
                },
                Match {
                    row: 1,
                    start: 0,
                    end: 3
                }
            ]
        );
        search.pattern = "Foo".into();
        assert!(search.find_matches().is_empty());
    }

    #[test]
    fn finds_cjk_and_emoji_graphemes() {
        let search = Search {
            lines: vec!["中文搜索：你好，🌈🙂 family 👩🏽‍💻".into()],
            pattern: "你好".into(),
            ..Search::default()
        };
        assert_eq!(
            search.find_matches()[0],
            Match {
                row: 0,
                start: 5,
                end: 7
            }
        );

        let search = Search {
            lines: vec!["中文搜索：你好，🌈🙂 family 👩🏽‍💻".into()],
            pattern: "👩🏽‍💻".into(),
            ..Search::default()
        };
        let found = search.find_matches();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].end - found[0].start, 1);
    }

    #[test]
    fn treats_combining_sequence_as_one_search_unit() {
        let search = Search {
            lines: vec!["cafe\u{301} cafe\u{301}".into()],
            pattern: "e\u{301}".into(),
            ..Search::default()
        };
        assert_eq!(search.find_matches().len(), 2);
    }

    #[test]
    fn cursor_distance_uses_terminal_cell_columns() {
        let search = Search {
            lines: vec!["中x x".into()],
            pattern: "x".into(),
            cursor_x: 4,
            ..Search::default()
        };
        let found = search.find_matches();
        assert_eq!(found[0].start, 3);
        assert_eq!(column_at(&search.lines[0], found[0].start), 4);
        assert_eq!(width("中"), 2);
        assert_eq!(width("👩🏽‍💻"), 2);
    }

    #[test]
    fn labels_skip_continuations_and_remain_ascii() {
        let search = Search {
            lines: vec!["cat 猫".into(), "cab".into()],
            pattern: "ca".into(),
            ..Search::default()
        };
        let matches = vec![
            Match {
                row: 0,
                start: 0,
                end: 2,
            },
            Match {
                row: 1,
                start: 0,
                end: 2,
            },
        ];
        let labels = search.assign_labels(&matches, "tabc");
        assert_eq!(labels.len(), 2);
        assert_eq!(labels[0].0, 'a');
    }
}
