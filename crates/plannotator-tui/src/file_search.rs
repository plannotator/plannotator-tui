//! Filename matching, independent of the filesystem and UI.
pub(crate) mod scan;

/// Lower ranks win: exact, substring, then ordered characters with fewer gaps.
/// Case folding is Unicode lowercase, not accent or normalization folding.
pub(crate) fn rank(name: &str, query: &str) -> Option<(u8, usize, usize, usize)> {
    let name = name.to_lowercase();
    let length = name.chars().count();
    if query.is_empty() {
        return Some((0, 0, 0, 0));
    }
    if name == query {
        return Some((0, 0, 0, length));
    }
    if let Some(start) = name.find(query) {
        return Some((1, 0, name.get(..start)?.chars().count(), length));
    }
    let mut wanted = query.chars();
    let mut next = wanted.next()?;
    let mut first = None;
    let mut matched = 0;
    for (index, ch) in name.chars().enumerate() {
        if ch != next {
            continue;
        }
        let start = *first.get_or_insert(index);
        matched += 1;
        match wanted.next() {
            Some(ch) => next = ch,
            None => return Some((2, index + 1 - start - matched, start, length)),
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::rank;

    #[test]
    fn exact_then_substring_then_ordered_characters() {
        assert!(rank("PRD", "prd") < rank("prd.md", "prd"));
        assert!(rank("prd.md", "prd") < rank("product-requirements.md", "prd"));
        assert!(rank("product-requirements.md", "prd").is_some());
        assert_eq!(rank("readme.md", "raedme"), None);
        assert!(rank("แผนงาน.md", "ผง").is_some());
        assert!(rank("ÉTUDE.md", "ét").is_some());
        assert!(rank("anything.md", "").is_some());
    }
}
