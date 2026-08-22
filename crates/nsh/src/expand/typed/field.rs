use bstr::BString;

use crate::pattern::{Pattern, PatternOptions};

#[derive(Clone, Debug)]
pub(super) struct Field {
    pub(super) bytes: BString,
    pub(super) regions: Vec<FieldRegion>,
    /// A quoted zero-width contribution at each byte boundary. Empty quotes
    /// can survive field splitting at the beginning, middle, or end of a
    /// word, so their sparse offsets are retained explicitly.
    pub(super) empty_anchors: Vec<usize>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct FieldRegion {
    pub(super) start: usize,
    pub(super) end: usize,
    pub(super) quoted: bool,
    pub(super) splittable: bool,
}

impl Default for Field {
    fn default() -> Self {
        Self {
            bytes: BString::new(Vec::new()),
            regions: Vec::new(),
            empty_anchors: Vec::new(),
        }
    }
}

impl Field {
    pub(super) fn from_bytes(
        bytes: &[u8],
        quoted: bool,
        splittable: bool,
        preserve_empty: bool,
    ) -> Self {
        let bytes = bytes
            .iter()
            .copied()
            .filter(|byte| *byte != 0)
            .collect::<Vec<_>>();
        let len = bytes.len();
        Self {
            bytes: BString::from(bytes),
            regions: (len != 0)
                .then_some(FieldRegion {
                    start: 0,
                    end: len,
                    quoted,
                    splittable,
                })
                .into_iter()
                .collect(),
            empty_anchors: (preserve_empty && len == 0)
                .then_some(0)
                .into_iter()
                .collect(),
        }
    }

    pub(super) fn append(&mut self, mut other: Self) {
        let boundary = self.bytes.len();
        for mut region in other.regions {
            region.start += boundary;
            region.end += boundary;
            self.push_region(region);
        }
        self.empty_anchors.extend(
            other
                .empty_anchors
                .into_iter()
                .map(|offset| boundary + offset),
        );
        self.empty_anchors.dedup();
        self.bytes.append(&mut other.bytes);
    }

    pub(super) fn slice(&self, range: std::ops::Range<usize>) -> Self {
        let mut result = Self {
            bytes: BString::from(&self.bytes[range.clone()]),
            regions: Vec::new(),
            empty_anchors: self
                .empty_anchors
                .iter()
                .copied()
                .filter(|offset| range.start <= *offset && *offset <= range.end)
                .map(|offset| offset - range.start)
                .collect(),
        };
        for region in &self.regions {
            let start = region.start.max(range.start);
            let end = region.end.min(range.end);
            if start < end {
                result.push_region(FieldRegion {
                    start: start - range.start,
                    end: end - range.start,
                    quoted: region.quoted,
                    splittable: region.splittable,
                });
            }
        }
        result
    }

    pub(super) fn anchor_empty(&mut self) {
        if self.bytes.is_empty() && self.empty_anchors.is_empty() {
            self.empty_anchors.push(self.bytes.len());
        }
    }

    pub(super) fn has_empty_anchor(&self, range: std::ops::RangeInclusive<usize>) -> bool {
        self.empty_anchors
            .iter()
            .any(|offset| range.contains(offset))
    }

    fn push_region(&mut self, region: FieldRegion) {
        if let Some(previous) = self.regions.last_mut()
            && previous.end == region.start
            && previous.quoted == region.quoted
            && previous.splittable == region.splittable
        {
            previous.end = region.end;
        } else {
            self.regions.push(region);
        }
    }

    fn region_at(&self, at: usize) -> Option<&FieldRegion> {
        let index = self.regions.partition_point(|region| region.end <= at);
        self.regions
            .get(index)
            .filter(|region| region.start <= at && at < region.end)
    }

    fn quoted_at(&self, at: usize) -> bool {
        self.region_at(at).is_some_and(|region| region.quoted)
    }

    pub(super) fn any_splittable(&self) -> bool {
        self.regions.iter().any(|region| region.splittable)
    }

    pub(super) fn range_is_splittable(&self, range: std::ops::Range<usize>) -> bool {
        let mut at = range.start;
        while at < range.end {
            let Some(region) = self.region_at(at) else {
                return false;
            };
            if !region.splittable {
                return false;
            }
            at = region.end.min(range.end);
        }
        at == range.end
    }

    pub(super) fn pattern(&self, options: PatternOptions) -> Pattern {
        let mut bytes = Vec::with_capacity(self.bytes.len());
        let mut quoted = Vec::with_capacity(self.bytes.len());
        let mut at = 0;

        while at < self.bytes.len() {
            if self.bytes[at] == b'\\' && !self.quoted_at(at) && at + 1 < self.bytes.len() {
                bytes.push(self.bytes[at + 1]);
                quoted.push(true);
                at += 2;
            } else {
                bytes.push(self.bytes[at]);
                quoted.push(self.quoted_at(at));
                at += 1;
            }
        }

        Pattern::new(BString::from(bytes), quoted).with_options(options)
    }
}
