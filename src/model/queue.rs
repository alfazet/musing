use anyhow::Result;
use rand::{prelude::*, seq::SliceRandom};
use std::{collections::HashSet, mem};

use crate::error::MyError;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Entry {
    pub queue_id: u32,
    pub db_id: u32,
}

impl From<(u32, u32)> for Entry {
    fn from((queue_id, db_id): (u32, u32)) -> Self {
        Entry { queue_id, db_id }
    }
}

struct Random {
    rng: SmallRng,
    ids: Vec<u32>,
}

#[derive(Default)]
pub struct Queue {
    list: Vec<Entry>,
    pos: Option<usize>,
    history: HashSet<u32>,
    next_id: u32,
    random: Option<Random>,
}

impl Default for Random {
    fn default() -> Self {
        Self {
            rng: SmallRng::from_os_rng(),
            ids: Vec::new(),
        }
    }
}

impl Random {
    pub fn new(mut ids: Vec<u32>) -> Self {
        let mut rng = SmallRng::from_os_rng();
        ids.shuffle(&mut rng);
        Self { rng, ids }
    }
}

impl Queue {
    fn find_by_id(&self, id: u32) -> Option<usize> {
        self.list.iter().position(|entry| entry.queue_id == id)
    }

    pub fn new() -> Self {
        Self::default()
    }

    pub fn db_id(&self, queue_id: u32) -> Option<u32> {
        let i = self.find_by_id(queue_id)?;
        Some(self.list[i].db_id)
    }

    pub fn current(&self) -> Option<Entry> {
        self.pos.map(|pos| self.list[pos])
    }

    pub fn as_inner(&self) -> &[Entry] {
        &self.list
    }

    pub fn reset_pos(&mut self) {
        let _ = self.pos.take();
    }

    pub fn add_current_to_history(&mut self) {
        if let Some(current) = self.current() {
            self.history.insert(current.queue_id);
        }
    }

    pub fn move_next(&mut self) -> Option<Entry> {
        match &mut self.random {
            Some(random) => {
                // move to the next random position or None if none are left
                self.pos = random.ids.pop().and_then(|id| self.find_by_id(id))
            }
            None => match &mut self.pos {
                Some(pos) if *pos < self.list.len() - 1 => *pos += 1,
                None if !self.list.is_empty() => self.pos = Some(0),
                _ => self.pos = None,
            },
        };

        self.current()
    }

    pub fn move_prev(&mut self) -> Option<Entry> {
        match &mut self.pos {
            Some(pos) if *pos > 0 => *pos -= 1,
            None if !self.list.is_empty() => self.pos = Some(self.list.len() - 1),
            _ => self.pos = None,
        };

        self.current()
    }

    pub fn move_to(&mut self, id: u32) -> Option<Entry> {
        // to prevent repetitions
        if let Some(random) = &mut self.random {
            random.ids.retain(|random_id| *random_id != id);
        };

        if let Some(pos) = self.find_by_id(id) {
            self.pos = Some(pos);
            self.current()
        } else {
            None
        }
    }

    pub fn add(&mut self, db_id: u32, pos: Option<usize>) {
        self.next_id += 1;
        let entry = Entry {
            queue_id: self.next_id,
            db_id,
        };

        match pos {
            Some(pos) if pos <= self.list.len() => self.list.insert(pos, entry),
            _ => self.list.push(entry),
        }
        if let Some(random) = &mut self.random {
            // insert into a random spot in constant time
            let random_pos = random.rng.random_range(0..random.ids.len());
            let temp = mem::replace(&mut random.ids[random_pos], entry.queue_id);
            random.ids.push(temp);
        }
    }

    /// Returns Some(true) if the removed song was currently playing
    /// Some(false) if not, and None if the song wasn't found.
    pub fn remove(&mut self, id: u32) -> Option<bool> {
        if let Some(random) = &mut self.random {
            random.ids.retain(|random_id| *random_id != id);
        }
        if let Some(removed_pos) = self.find_by_id(id) {
            self.list.remove(removed_pos);
            if let Some(cur_pos) = self.pos {
                if cur_pos == removed_pos {
                    self.pos = None;
                    Some(true)
                } else {
                    if cur_pos > removed_pos {
                        self.pos = Some(cur_pos - 1);
                    }
                    Some(false)
                }
            } else {
                Some(false)
            }
        } else {
            None
        }
    }

    pub fn clear(&mut self) {
        self.list.clear();
        self.history.clear();
        let _ = self.random.take();
    }

    pub fn toggle_random(&mut self) {
        if let Some(random) = &self.random {
            let _ = self.random.take();
        } else {
            let not_played_ids: Vec<_> = self
                .list
                .clone()
                .into_iter()
                .filter(|entry| {
                    !self.history.contains(&entry.queue_id)
                        && self
                            .current()
                            .map(|cur_entry| entry.queue_id != cur_entry.queue_id)
                            .unwrap_or(true)
                })
                .map(|entry| entry.queue_id)
                .collect();
            self.random = Some(Random::new(not_played_ids));
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn add_and_remove() {
        let mut queue = Queue::new();
        queue.add(1001, None);
        queue.add(1002, Some(0));
        queue.add(1003, None);
        queue.add(1004, Some(2));
        let expected = &[
            (2, 1002).into(),
            (1, 1001).into(),
            (4, 1004).into(),
            (3, 1003).into(),
        ];
        assert_eq!(queue.as_inner(), expected);

        queue.remove(4);
        queue.remove(2137);
        queue.add(1005, None);
        queue.remove(2);
        queue.add(1006, Some(1));
        let expected = &[
            (1, 1001).into(),
            (6, 1006).into(),
            (3, 1003).into(),
            (5, 1005).into(),
        ];
        assert_eq!(queue.as_inner(), expected);
    }

    #[test]
    fn traversing() {
        let mut queue = Queue::new();
        let n = 5;
        for i in 1001..=(1000 + n) {
            queue.add(i, None);
        }

        queue.move_next();
        queue.move_next();
        assert_eq!(queue.current(), Some((2, 1002).into()));
        queue.move_next();
        queue.move_prev();
        assert_eq!(queue.current(), Some((2, 1002).into()));
        queue.move_next();
        queue.move_next();
        assert_eq!(queue.current(), Some((4, 1004).into()));
        queue.move_next();
        queue.move_next();
        assert_eq!(queue.current(), None);
        queue.move_prev();
        assert_eq!(queue.current(), Some((5, 1005).into()));
        queue.move_next();
        queue.move_next();
        assert_eq!(queue.current(), Some((1, 1001).into()));
    }

    #[test]
    fn random() {
        let mut queue = Queue::new();
        let n = 100;
        for i in 1000..(1000 + n) {
            queue.add(i, None);
        }

        let mut seen = HashSet::new();
        queue.move_next();
        let cur_on_toggle = queue.current();
        seen.insert(cur_on_toggle.unwrap().queue_id);
        queue.toggle_random();
        for i in 0..(n - 1) {
            let cur = queue.current();
            if i == 0 {
                // check that toggling random doesn't "move" the current song
                assert_eq!(cur, cur_on_toggle);
            } else {
                assert!(cur.is_some() && !seen.contains(&cur.unwrap().queue_id));
            }
            seen.insert(cur.unwrap().queue_id);
            queue.move_next();
        }
    }
}
