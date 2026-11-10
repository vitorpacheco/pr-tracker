//! Which items show, in what groups, and what the arrow keys select.

use std::path::PathBuf;

use pr_tracker_core::provider::{Item, Kind, Relation};

/// An item with what the list needs alongside it.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub item: Item,
    pub key: String,
    pub reference: String,
    pub clone: Option<PathBuf>,
    pub worktree: Option<PathBuf>,
}

pub struct Group<'a> {
    pub repo: &'a str,
    pub instance: &'a str,
    pub rows: Vec<&'a Row>,
}

/// `relation` of [`Relation::NONE`] and an empty `instance` match everything.
pub fn filter<'a>(rows: &'a [Row], kind: Kind, relation: Relation, query: &str, instance: &str) -> Vec<&'a Row> {
    let query = query.trim().to_lowercase();
    rows.iter()
        .filter(|row| {
            let item = &row.item;
            item.kind == kind
                && (relation == Relation::NONE || item.relations.contains(relation))
                && (instance.is_empty() || item.instance == instance)
                && (query.is_empty()
                    || format!("{} {} {} {}", item.title, item.repo, item.author, row.reference).to_lowercase().contains(&query))
        })
        .collect()
}

/// Input is ordered by recency. That order is kept within each repository, and
/// groups are ordered by their most recently updated item.
pub fn group_by_repository<'a>(rows: &[&'a Row]) -> Vec<Group<'a>> {
    let mut groups: Vec<Group<'a>> = Vec::new();
    for &row in rows {
        let item = &row.item;
        let same = |group: &Group<'a>| group.rows[0].item.instance == item.instance && group.rows[0].item.host == item.host && group.repo == item.repo;
        match groups.iter_mut().find(|group| same(group)) {
            Some(group) => group.rows.push(row),
            None => groups.push(Group { repo: &item.repo, instance: &item.instance, rows: vec![row] }),
        }
    }
    groups
}

/// The key `delta` rows away from `selected`, staying within the list. With
/// nothing selected, either direction picks the first row.
pub fn next_selection<'a>(keys: &[&'a str], selected: &str, delta: isize) -> Option<&'a str> {
    if keys.is_empty() {
        return None;
    }
    let start = match keys.iter().position(|key| *key == selected) {
        Some(index) => index as isize,
        None if delta > 0 => -1,
        None => 1,
    };
    let index = (start + delta).clamp(0, keys.len() as isize - 1);
    Some(keys[index as usize])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(instance: &str, repo: &str, number: i64, kind: Kind, relations: Relation) -> Row {
        let item = Item {
            kind,
            instance: instance.into(),
            repo: repo.into(),
            number,
            title: format!("Título {number}"),
            author: "ana".into(),
            relations,
            ..Item::default()
        };
        Row { key: item.key(), reference: item.reference(), item, clone: None, worktree: None }
    }

    fn rows() -> Vec<Row> {
        vec![
            row("work", "org/api", 3, Kind::Pr, Relation::REVIEW_REQUESTED),
            row("home", "me/site", 9, Kind::Pr, Relation::AUTHORED),
            row("work", "org/api", 1, Kind::Pr, Relation::AUTHORED | Relation::ASSIGNED),
            row("work", "org/api", 7, Kind::Issue, Relation::MENTIONED),
        ]
    }

    #[test]
    fn filters_combine_kind_relation_instance_and_query() {
        let rows = rows();
        let numbers = |found: Vec<&Row>| found.iter().map(|row| row.item.number).collect::<Vec<_>>();
        assert_eq!(numbers(filter(&rows, Kind::Pr, Relation::NONE, "", "")), [3, 9, 1]);
        assert_eq!(numbers(filter(&rows, Kind::Pr, Relation::AUTHORED, "", "")), [9, 1]);
        assert_eq!(numbers(filter(&rows, Kind::Pr, Relation::AUTHORED, "", "work")), [1]);
        assert_eq!(numbers(filter(&rows, Kind::Pr, Relation::NONE, "  ME/SITE ", "")), [9]);
        assert_eq!(numbers(filter(&rows, Kind::Pr, Relation::NONE, "#3", "")), [3]);
        assert_eq!(numbers(filter(&rows, Kind::Issue, Relation::NONE, "", "")), [7]);
    }

    #[test]
    fn groups_keep_recency_order() {
        let rows = rows();
        let visible = filter(&rows, Kind::Pr, Relation::NONE, "", "");
        let groups = group_by_repository(&visible);
        let shape: Vec<_> = groups.iter().map(|group| (group.repo, group.instance, group.rows.len())).collect();
        assert_eq!(shape, [("org/api", "work", 2), ("me/site", "home", 1)]);
        assert_eq!(groups[0].rows.iter().map(|row| row.item.number).collect::<Vec<_>>(), [3, 1]);
    }

    #[test]
    fn selection_moves_within_bounds() {
        let keys = ["a", "b", "c"];
        assert_eq!(next_selection(&keys, "a", 1), Some("b"));
        assert_eq!(next_selection(&keys, "c", 1), Some("c"));
        assert_eq!(next_selection(&keys, "a", -1), Some("a"));
        assert_eq!(next_selection(&keys, "", 1), Some("a"));
        assert_eq!(next_selection(&keys, "gone", -1), Some("a"));
        assert_eq!(next_selection(&[], "a", 1), None);
    }
}
