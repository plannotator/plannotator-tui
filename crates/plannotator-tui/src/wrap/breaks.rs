//! Where a line may break: between words, and between CJK characters, which have no spaces
//! to break at. Basic kinsoku keeps opening punctuation with the text after it and closing
//! punctuation with the text before it.

use super::Cell;

/// Punctuation that must not end a row, because it belongs to the text after it.
fn opens_text(ch: char) -> bool {
    matches!(ch, '（' | '「' | '『' | '【' | '《' | '〈' | '〔' | '［' | '｛' | '“' | '‘')
}

/// Punctuation that must not start a row, because it belongs to the text before it.
fn closes_text(ch: char) -> bool {
    matches!(
        ch,
        '，' | '。'
            | '、'
            | '：'
            | '；'
            | '！'
            | '？'
            | '）'
            | '」'
            | '』'
            | '】'
            | '》'
            | '〉'
            | '〕'
            | '］'
            | '｝'
            | '”'
            | '’'
            | '…'
            | '—'
    )
}

/// Length of the unbreakable run at the front of `cells`: a run of whitespace, or a word or
/// single wide character together with any punctuation that must stay attached to it. Wide
/// characters break individually because CJK text has no spaces to break at.
pub(super) fn token_len(cells: &[Cell]) -> usize {
    let is_space = |cell: &Cell| cell.ch.is_whitespace();
    let is_wide = |cell: &Cell| cell.width > 1;
    if cells.first().is_some_and(is_space) {
        return cells.iter().take_while(|cell| is_space(cell)).count();
    }
    let mut len = cells.iter().take_while(|cell| opens_text(cell.ch)).count();
    if cells.get(len).is_some_and(is_wide) {
        len += 1;
    } else {
        let word = cells.get(len..).unwrap_or_default();
        len +=
            word.iter().take_while(|cell| !is_space(cell) && !is_wide(cell) && !opens_text(cell.ch)).count();
    }
    let tail = cells.get(len..).unwrap_or_default();
    len += tail.iter().take_while(|cell| closes_text(cell.ch)).count();
    len.max(1)
}

#[cfg(test)]
mod tests {
    use ratatui::text::Line;

    use super::super::tests::{identity, plain};
    use super::super::wrap_line;

    #[test]
    fn cjk_text_breaks_between_characters_and_not_after_the_marker() {
        let source = "- 甲乙丙丁戊己庚辛壬癸";
        let rows = wrap_line(&Line::from(source), &identity(source.chars().count()), 10);
        assert_eq!(plain(&rows), ["- 甲乙丙丁", "戊己庚辛壬", "癸"]);
    }

    #[test]
    fn cjk_punctuation_stays_with_its_text() {
        let source = "甲乙丙，丁（戊）";
        let rows = wrap_line(&Line::from(source), &identity(source.chars().count()), 6);
        assert_eq!(plain(&rows), ["甲乙", "丙，丁", "（戊）"]);
    }

    #[test]
    fn latin_words_inside_cjk_stay_whole() {
        let source = "甲乙Dandadan丙";
        let rows = wrap_line(&Line::from(source), &identity(source.chars().count()), 8);
        assert_eq!(plain(&rows), ["甲乙", "Dandadan", "丙"]);
    }
}
