//! Identificaties kept for checking repeats and references after a read.
//!
//! The extract stores each object type in identificatie order, so identificaties usually
//! arrive ascending. [`Ids`] then keeps only varint-encoded differences, a byte or two each
//! instead of eight, and knows there are no repeats. Identificaties out of order switch it
//! to a plain list.

/// Identificaties in arrival order.
#[derive(Debug, Default)]
pub struct Ids {
    /// Varint differences while every identificatie exceeded the one before.
    deltas: Vec<u8>,
    last: i64,
    count: usize,
    /// All identificaties, once one arrived out of order.
    plain: Option<Vec<i64>>,
}

impl Ids {
    pub fn push(&mut self, id: i64) {
        if let Some(plain) = &mut self.plain {
            plain.push(id);
            return;
        }
        if self.count > 0 && id <= self.last {
            let mut plain: Vec<i64> = self.ascending().collect();
            plain.push(id);
            self.plain = Some(plain);
            self.deltas = Vec::new();
            return;
        }
        let mut delta = (id - if self.count == 0 { 0 } else { self.last }) as u64;
        loop {
            let byte = (delta & 0x7f) as u8;
            delta >>= 7;
            if delta == 0 {
                self.deltas.push(byte);
                break;
            }
            self.deltas.push(byte | 0x80);
        }
        self.last = id;
        self.count += 1;
    }

    pub fn extend(&mut self, ids: impl IntoIterator<Item = i64>) {
        ids.into_iter().for_each(|id| self.push(id));
    }

    pub fn len(&self) -> usize {
        self.plain.as_ref().map_or(self.count, Vec::len)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The identificaties in ascending order with repeats kept, or the first repeat.
    pub fn sorted_unique(self) -> Result<Sorted, i64> {
        match self.plain {
            None => Ok(Sorted {
                deltas: self.deltas,
                count: self.count,
            }),
            Some(mut plain) => {
                plain.sort_unstable();
                if let Some(pair) = plain.windows(2).find(|w| w[0] == w[1]) {
                    return Err(pair[0]);
                }
                let mut ids = Ids::default();
                ids.extend(plain);
                Ok(Sorted {
                    deltas: ids.deltas,
                    count: ids.count,
                })
            }
        }
    }

    /// Decodes the differences; only valid before switching to a plain list.
    fn ascending(&self) -> impl Iterator<Item = i64> + '_ {
        decode(&self.deltas, self.count)
    }
}

/// Distinct identificaties in ascending order.
#[derive(Debug, Default)]
pub struct Sorted {
    deltas: Vec<u8>,
    count: usize,
}

impl Sorted {
    pub fn iter(&self) -> impl Iterator<Item = i64> + '_ {
        decode(&self.deltas, self.count)
    }

    /// Identificaties of `references` (ascending, distinct) missing from `self`.
    pub fn missing<'a>(&'a self, references: &'a Sorted) -> impl Iterator<Item = i64> + 'a {
        let mut ids = self.iter().peekable();
        references.iter().filter(move |&reference| {
            while ids.next_if(|&id| id < reference).is_some() {}
            ids.peek() != Some(&reference)
        })
    }
}

/// Builds a [`Sorted`] from identificaties in any order, dropping repeats.
pub fn distinct(mut ids: Vec<i64>) -> Sorted {
    ids.sort_unstable();
    ids.dedup();
    let mut log = Ids::default();
    log.extend(ids);
    Sorted {
        deltas: log.deltas,
        count: log.count,
    }
}

fn decode(deltas: &[u8], count: usize) -> impl Iterator<Item = i64> + '_ {
    let mut at = 0;
    let mut value = 0i64;
    (0..count).map(move |_| {
        let mut delta = 0u64;
        let mut shift = 0;
        loop {
            let byte = deltas[at];
            at += 1;
            delta |= u64::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                break;
            }
            shift += 7;
        }
        value += delta as i64;
        value
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascending_ids_round_trip_and_disorder_falls_back() {
        let mut ids = Ids::default();
        ids.extend([3, 10, 1 << 50, (1 << 50) + 1]);
        assert_eq!(ids.len(), 4);
        let sorted = ids.sorted_unique().unwrap();
        assert_eq!(
            sorted.iter().collect::<Vec<_>>(),
            [3, 10, 1 << 50, (1 << 50) + 1]
        );

        let mut ids = Ids::default();
        ids.extend([5, 9, 2, 7]);
        assert_eq!(
            ids.sorted_unique().unwrap().iter().collect::<Vec<_>>(),
            [2, 5, 7, 9]
        );
        let mut ids = Ids::default();
        ids.extend([5, 9, 5]);
        assert_eq!(ids.sorted_unique().unwrap_err(), 5);

        let present = distinct(vec![1, 4, 9]);
        let references = distinct(vec![9, 2, 4, 4]);
        assert_eq!(present.missing(&references).collect::<Vec<_>>(), [2]);
    }
}
