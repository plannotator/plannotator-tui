//! Delivery coverage and recoverable finished reviews. Changes commit to memory only
//! after the record has been saved, so a failed archive or restore leaves it intact.

use std::collections::HashSet;

use anyhow::{Context, Result};
use plannotator_tui_schema::Annotation;
use time::{Duration, OffsetDateTime, UtcOffset, format_description::well_known::Rfc3339};

use super::{Delivered, Store};
use crate::doc::Document;

/// Parsing stays lenient: any RFC 3339 offset and sub-second precision is accepted.
fn parse_time(value: &str) -> Option<OffsetDateTime> {
    OffsetDateTime::parse(value, &Rfc3339).ok()
}

/// Every timestamp the record writes has one shape, `YYYY-MM-DDTHH:MM:SS.mmmZ`, so a
/// record never mixes precisions (see `archive::iso_millis`). Finer digits are dropped.
fn format_millis(at: OffsetDateTime) -> Result<String> {
    let at = at.checked_to_offset(UtcOffset::UTC).context("annotation time out of range")?;
    Ok(format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        at.year(),
        u8::from(at.month()),
        at.day(),
        at.hour(),
        at.minute(),
        at.second(),
        at.millisecond()
    ))
}

/// The millisecond at or after `at`, so a stored copy never sorts before the instant it
/// stands for when that instant carried finer digits.
fn ceil_millis(at: OffsetDateTime) -> Result<OffsetDateTime> {
    let below = i64::from(at.nanosecond() % 1_000_000);
    if below == 0 {
        return Ok(at);
    }
    at.checked_add(Duration::nanoseconds(1_000_000 - below)).context("advancing annotation time")
}

pub(super) fn timestamp() -> Result<String> {
    format_millis(OffsetDateTime::now_utc())
}

impl Store {
    /// Compare persisted review data, independent of resolution against the document.
    pub(crate) fn same_review(&self, other: &Self) -> bool {
        self.annotations == other.annotations
            && self.deliveries == other.deliveries
            && self.archived == other.archived
    }

    fn last_delivery(&self, id: &str) -> Option<&Delivered> {
        self.deliveries.iter().rev().find(|d| d.annotation_ids.iter().any(|sent| sent == id))
    }

    /// Coverage belongs to each annotation's last successful send, not the last batch.
    /// A delivery whose time cannot be read covers nothing, so the annotation stays
    /// pending rather than silently hiding feedback. An annotation whose own `updated_at`
    /// cannot be read is the other way round: once it has been in any delivery it counts as
    /// sent, because "pending forever" would resend it on every send and never let it be
    /// archived. The next edit rewrites the timestamp and makes it pending again.
    pub(crate) fn is_pending(&self, annotation: &Annotation) -> bool {
        let Some(delivery) = self.last_delivery(&annotation.id) else { return true };
        match (parse_time(&annotation.updated_at), parse_time(&delivery.at)) {
            (Some(updated), Some(sent)) => updated > sent,
            (None, _) => false,
            (Some(_), None) => true,
        }
    }

    pub(crate) fn all_delivered(&self) -> bool {
        !self.annotations.is_empty() && self.annotations.iter().all(|a| !self.is_pending(a))
    }

    /// Replace a body, advancing the existing timestamp beyond its last send even when
    /// the clock has not ticked (or has moved backwards) since that send. One millisecond
    /// is the smallest step the stored shape can represent.
    pub(crate) fn edit_body(&mut self, id: &str, body: String) -> Result<bool> {
        let Some(annotation) = self.annotations.iter().find(|a| a.id == id) else { return Ok(false) };
        if annotation.body == body {
            return Ok(false);
        }
        let previous =
            [parse_time(&annotation.updated_at), self.last_delivery(id).and_then(|d| parse_time(&d.at))]
                .into_iter()
                .flatten()
                .max();
        let mut now = OffsetDateTime::now_utc();
        if let Some(previous) = previous {
            now = now
                .max(previous.checked_add(Duration::milliseconds(1)).context("advancing annotation time")?);
        }
        let updated_at = format_millis(now)?;
        let mut next = self.clone();
        if let Some(annotation) = next.annotations.iter_mut().find(|a| a.id == id) {
            annotation.body = body;
            annotation.updated_at = updated_at;
        }
        next.save()?;
        *self = next;
        Ok(true)
    }

    /// Record only ids that actually appeared in the delivered feedback body.
    pub(crate) fn record_delivery(&mut self, target: &str, annotation_ids: &[String]) -> Result<()> {
        if annotation_ids.is_empty() {
            return Ok(());
        }
        let updated = self
            .annotations
            .iter()
            .filter(|a| annotation_ids.contains(&a.id))
            .filter_map(|a| parse_time(&a.updated_at))
            .max();
        let now = updated.map_or_else(OffsetDateTime::now_utc, |at| at.max(OffsetDateTime::now_utc()));
        let mut next = self.clone();
        next.deliveries.push(Delivered {
            at: format_millis(ceil_millis(now)?)?,
            target: target.to_owned(),
            annotation_ids: annotation_ids.to_vec(),
        });
        next.save()?;
        *self = next;
        Ok(())
    }

    pub(crate) fn archived(&self) -> &[Annotation] {
        &self.archived
    }

    /// Finish delivered, unchanged annotations, including ones whose quote is now gone.
    /// Pending annotations stay active. The returned ids are the undo operation.
    pub(crate) fn archive_sent(&mut self) -> Result<Vec<String>> {
        let ids: Vec<String> =
            self.annotations.iter().filter(|a| !self.is_pending(a)).map(|a| a.id.clone()).collect();
        if ids.is_empty() {
            return Ok(ids);
        }
        let selected: HashSet<&str> = ids.iter().map(String::as_str).collect();
        anyhow::ensure!(
            !self.archived.iter().any(|a| selected.contains(a.id.as_str())),
            "cannot archive: an annotation with the same id is already archived"
        );
        let mut next = self.clone();
        let active = std::mem::take(&mut next.annotations);
        let resolved = std::mem::take(&mut next.resolved);
        for (annotation, resolution) in active.into_iter().zip(resolved) {
            if selected.contains(annotation.id.as_str()) {
                next.archived.push(annotation);
            } else {
                next.annotations.push(annotation);
                next.resolved.push(resolution);
            }
        }
        next.save()?;
        *self = next;
        Ok(ids)
    }

    /// Restore without editing timestamps or delivery history. A conflicting active id
    /// is left alone, with the archived copy retained for recovery.
    pub(crate) fn restore_archived(&mut self, doc: &Document, ids: &[String]) -> Result<usize> {
        let mut next = self.clone();
        let archived = std::mem::take(&mut next.archived);
        let mut restored = 0;
        for annotation in archived {
            if ids.contains(&annotation.id) && !next.annotations.iter().any(|a| a.id == annotation.id) {
                next.annotations.push(annotation);
                restored += 1;
            } else {
                next.archived.push(annotation);
            }
        }
        if restored > 0 {
            next.resolve_all(doc);
            next.save()?;
            *self = next;
        }
        Ok(restored)
    }
}

#[cfg(test)]
mod tests;
