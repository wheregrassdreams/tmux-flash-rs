use std::cell::RefCell;
use std::collections::HashSet;
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
    lines: Vec<IndexedLine>,
    pub pattern: String,
    pub cursor_x: usize,
    pub cursor_y: usize,
    match_cache: RefCell<MatchCache>,
}

#[derive(Clone, Debug, Default)]
struct MatchCache {
    pattern: String,
    case_insensitive: bool,
    cursor_x: usize,
    cursor_y: usize,
    matches: Vec<Match>,
    #[cfg(test)]
    full_scans: usize,
}

#[derive(Clone, Debug, Default)]
struct IndexedLine {
    text: String,
    grapheme_offsets: Vec<usize>,
    cell_columns: Vec<usize>,
    lowercase: Vec<String>,
}

impl IndexedLine {
    fn new(text: String) -> Self {
        let mut grapheme_offsets = Vec::new();
        let mut cell_columns = vec![0];
        let mut lowercase_clusters = Vec::new();
        let mut column = 0;

        for (offset, cluster) in text.grapheme_indices(true) {
            grapheme_offsets.push(offset);
            column += grapheme_width(cluster);
            cell_columns.push(column);
            lowercase_clusters.push(lowercase(cluster));
        }
        grapheme_offsets.push(text.len());

        Self {
            text,
            grapheme_offsets,
            cell_columns,
            lowercase: lowercase_clusters,
        }
    }

    fn grapheme_count(&self) -> usize {
        self.grapheme_offsets.len().saturating_sub(1)
    }

    fn grapheme(&self, index: usize) -> &str {
        &self.text[self.grapheme_offsets[index]..self.grapheme_offsets[index + 1]]
    }
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
    pub fn new(lines: Vec<String>) -> Self {
        Self {
            lines: lines.into_iter().map(IndexedLine::new).collect(),
            ..Self::default()
        }
    }

    pub fn iter_lines(&self) -> impl Iterator<Item = &str> {
        self.lines.iter().map(|line| line.text.as_str())
    }

    pub fn line(&self, row: usize) -> &str {
        &self.lines[row].text
    }

    pub fn grapheme_count(&self, row: usize) -> usize {
        self.lines[row].grapheme_count()
    }

    pub fn grapheme(&self, row: usize, index: usize) -> &str {
        self.lines[row].grapheme(index)
    }

    pub fn find_matches(&self) -> Vec<Match> {
        let needle: Vec<&str> = self.pattern.graphemes(true).collect();
        let case_insensitive = self.pattern.chars().all(|ch| !ch.is_uppercase());
        let mut cache = self.match_cache.borrow_mut();
        let cursor_unchanged = cache.cursor_x == self.cursor_x && cache.cursor_y == self.cursor_y;

        if cursor_unchanged
            && cache.pattern == self.pattern
            && cache.case_insensitive == case_insensitive
        {
            return cache.matches.clone();
        }

        let previous: Vec<&str> = cache.pattern.graphemes(true).collect();
        let extends_previous = cursor_unchanged
            && !cache.pattern.is_empty()
            && cache.case_insensitive == case_insensitive
            && needle.len() > previous.len()
            && needle.starts_with(&previous);

        if extends_previous {
            let suffix = &needle[previous.len()..];
            let folded_suffix: Vec<String> = if case_insensitive {
                suffix.iter().map(|cluster| lowercase(cluster)).collect()
            } else {
                Vec::new()
            };
            cache.matches.retain_mut(|found| {
                let line = &self.lines[found.row];
                let matches_suffix = found.end + suffix.len() <= line.grapheme_count()
                    && suffix.iter().enumerate().all(|(offset, cluster)| {
                        let index = found.end + offset;
                        if case_insensitive {
                            line.lowercase[index] == folded_suffix[offset]
                        } else {
                            line.grapheme(index) == *cluster
                        }
                    });
                if matches_suffix {
                    found.end += suffix.len();
                }
                matches_suffix
            });
        } else {
            #[cfg(test)]
            {
                cache.full_scans += 1;
            }
            cache.matches = self.scan_matches(&needle, case_insensitive);
        }

        cache.pattern.clone_from(&self.pattern);
        cache.case_insensitive = case_insensitive;
        cache.cursor_x = self.cursor_x;
        cache.cursor_y = self.cursor_y;
        cache.matches.clone()
    }

    fn scan_matches(&self, needle: &[&str], case_insensitive: bool) -> Vec<Match> {
        if needle.is_empty() {
            return Vec::new();
        }
        let folded_needle: Vec<String> = if case_insensitive {
            needle.iter().map(|cluster| lowercase(cluster)).collect()
        } else {
            Vec::new()
        };
        let mut ranked_matches = Vec::new();

        for (row, line) in self.lines.iter().enumerate() {
            let grapheme_count = line.grapheme_count();
            if needle.len() > grapheme_count {
                continue;
            }

            let row_distance = row.abs_diff(self.cursor_y);
            for start in 0..=grapheme_count - needle.len() {
                let matched = if case_insensitive {
                    line.lowercase[start..start + needle.len()] == folded_needle
                } else {
                    needle
                        .iter()
                        .enumerate()
                        .all(|(offset, cluster)| line.grapheme(start + offset) == *cluster)
                };
                if matched {
                    let column_distance = line.cell_columns[start].abs_diff(self.cursor_x);
                    ranked_matches.push((
                        row_distance,
                        column_distance,
                        Match {
                            row,
                            start,
                            end: start + needle.len(),
                        },
                    ));
                }
            }
        }

        ranked_matches
            .sort_by_key(|(row_distance, column_distance, _)| (*row_distance, *column_distance));
        ranked_matches
            .into_iter()
            .map(|(_, _, found)| found)
            .collect()
    }

    pub fn assign_labels(&self, matches: &[Match], alphabet: &str) -> Vec<(char, Match)> {
        let case_insensitive = self.pattern.chars().all(|ch| !ch.is_uppercase());
        let mut continuations = HashSet::new();
        for found in matches {
            let line = &self.lines[found.row];
            if found.end < line.grapheme_count() {
                continuations.insert(if case_insensitive {
                    line.lowercase[found.end].as_str()
                } else {
                    line.grapheme(found.end)
                });
            }
        }

        alphabet
            .chars()
            .filter(|label| {
                let label = label.to_string();
                if case_insensitive {
                    !continuations.contains(lowercase(&label).as_str())
                } else {
                    !continuations.contains(label.as_str())
                }
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
        let mut search = Search::new(vec!["First foo".into(), "foo First".into()]);
        search.pattern = "foo".into();
        search.cursor_x = 4;
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
        let mut search = Search::new(vec!["中文搜索：你好，🌈🙂 family 👩🏽‍💻".into()]);
        search.pattern = "你好".into();
        assert_eq!(
            search.find_matches()[0],
            Match {
                row: 0,
                start: 5,
                end: 7
            }
        );

        search.pattern = "👩🏽‍💻".into();
        let found = search.find_matches();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].end - found[0].start, 1);
    }

    #[test]
    fn treats_combining_sequence_as_one_search_unit() {
        let mut search = Search::new(vec!["cafe\u{301} cafe\u{301}".into()]);
        search.pattern = "e\u{301}".into();
        assert_eq!(search.find_matches().len(), 2);
    }

    #[test]
    fn cursor_distance_uses_terminal_cell_columns() {
        let line = "中x x";
        let mut search = Search::new(vec![line.into()]);
        search.pattern = "x".into();
        search.cursor_x = 4;
        let found = search.find_matches();
        assert_eq!(found[0].start, 3);
        assert_eq!(column_at(line, found[0].start), 4);
        assert_eq!(width("中"), 2);
        assert_eq!(width("👩🏽‍💻"), 2);
    }

    #[test]
    fn labels_skip_continuations_and_remain_ascii() {
        let mut search = Search::new(vec!["cat 猫".into(), "cab".into()]);
        search.pattern = "ca".into();
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

    #[test]
    fn extending_pattern_filters_previous_candidates_and_backspace_rescans() {
        let mut search = Search::new(vec![
            "foobar foo bar".into(),
            "foobaz".into(),
            "FOOBAR".into(),
        ]);

        search.pattern = "foo".into();
        let foo_matches = search.find_matches();
        assert_eq!(foo_matches.len(), 4);
        assert_eq!(search.match_cache.borrow().full_scans, 1);

        search.pattern.push('b');
        let foob_matches = search.find_matches();
        assert_eq!(foob_matches.len(), 3);
        assert!(
            foob_matches
                .iter()
                .all(|found| found.end - found.start == 4)
        );
        assert_eq!(search.match_cache.borrow().full_scans, 1);

        search.pattern.pop();
        assert_eq!(search.find_matches(), foo_matches);
        assert_eq!(search.match_cache.borrow().full_scans, 2);
    }

    #[test]
    fn first_character_and_case_mode_changes_use_full_scan() {
        let mut search = Search::new(vec!["foo Foo FOO".into()]);
        search.pattern = "f".into();
        assert_eq!(search.find_matches().len(), 3);
        assert_eq!(search.match_cache.borrow().full_scans, 1);

        search.pattern.push('O');
        assert_eq!(search.find_matches().len(), 0);
        assert_eq!(search.match_cache.borrow().full_scans, 2);
    }
}
