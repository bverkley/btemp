//! Fixed-size ring buffers of recent samples per series (for sparklines).

use std::collections::HashMap;

/// Ring buffer of `f64` samples (temperatures in °C).
#[derive(Debug, Clone)]
pub struct RingBuffer {
    cap: usize,
    buf: std::collections::VecDeque<f64>,
}

impl RingBuffer {
    pub fn new(cap: usize) -> Self {
        Self {
            cap: cap.max(1),
            buf: std::collections::VecDeque::new(),
        }
    }

    pub fn push(&mut self, v: f64) {
        if self.buf.len() >= self.cap {
            self.buf.pop_front();
        }
        self.buf.push_back(v);
    }

    /// Values in chronological order, length <= cap.
    pub fn as_slice(&self) -> Vec<f64> {
        self.buf.iter().copied().collect()
    }

    /// Last value if any.
    pub fn last(&self) -> Option<f64> {
        self.buf.back().copied()
    }
}

/// One history entry per series id.
#[derive(Debug, Default)]
pub struct History {
    cap: usize,
    inner: HashMap<String, RingBuffer>,
}

impl History {
    pub fn new(cap: usize) -> Self {
        Self {
            cap,
            inner: HashMap::new(),
        }
    }

    pub fn record(&mut self, id: &str, value_c: f64) {
        self.inner
            .entry(id.to_string())
            .or_insert_with(|| RingBuffer::new(self.cap))
            .push(value_c);
    }

    pub fn buffer(&self, id: &str) -> Option<&RingBuffer> {
        self.inner.get(id)
    }

    pub fn ensure_series(&mut self, id: &str) {
        self.inner
            .entry(id.to_string())
            .or_insert_with(|| RingBuffer::new(self.cap));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_drops_oldest() {
        let mut r = RingBuffer::new(3);
        r.push(1.0);
        r.push(2.0);
        r.push(3.0);
        r.push(4.0);
        assert_eq!(r.as_slice(), vec![2.0, 3.0, 4.0]);
    }

    #[test]
    fn history_per_id() {
        let mut h = History::new(10);
        h.record("a", 40.0);
        h.record("a", 41.0);
        h.record("b", 30.0);
        assert_eq!(h.buffer("a").unwrap().as_slice(), vec![40.0, 41.0]);
    }
}
